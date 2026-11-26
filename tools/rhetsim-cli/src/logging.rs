//! Simulator log and execution-trace output.
//!
//! Two `tracing` layers are installed:
//!
//! * a **log** layer (stderr or `--log-file`) at `--log-level`, with the
//!   `rhetsim::trace::*` targets filtered out;
//! * a **trace** layer (stderr or `--trace-file`) that only enables the
//!   categories selected with `--trace`.
//!
//! Guest UART output is deliberately not part of this module.

use std::fs::File;
use std::io::{self, IsTerminal};
use std::sync::Arc;

use rhetsim_core::diag::{LogLevel, TraceCategory};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;

/// Log targets that belong to the simulator. The `rhetsim` prefix covers
/// library module targets such as `rhetsim_runtime::boot`.
const LOG_TARGETS: &[&str] = &[
    "cli",
    "platform",
    "boot",
    "loader",
    "console",
    "sched",
    "rhetsim",
];

#[derive(Debug, Clone)]
pub struct Options {
    pub level: LogLevel,
    pub log_file: Option<String>,
    pub trace: Vec<TraceCategory>,
    pub trace_file: Option<String>,
}

impl Options {
    pub fn trace_enabled(&self, category: TraceCategory) -> bool {
        self.trace.contains(&category)
    }
}

/// Parse the diagnostic flags and install the global subscriber.
pub fn init(args: &[String]) -> Result<Options, String> {
    let opts = parse(args)?;
    install(&opts)?;
    Ok(opts)
}

fn parse(args: &[String]) -> Result<Options, String> {
    let level = match flag_value(args, "--log-level")? {
        Some(level) => LogLevel::parse(level).ok_or_else(|| {
            format!("invalid log level '{level}' (expected error|warn|info|debug|trace)")
        })?,
        None => LogLevel::Info,
    };

    let trace = match flag_value(args, "--trace")? {
        Some(list) => TraceCategory::parse_list(list)?,
        None => Vec::new(),
    };

    let log_file = flag_value(args, "--log-file")?.map(str::to_string);
    let trace_file = flag_value(args, "--trace-file")?.map(str::to_string);

    if log_file.is_some() && log_file == trace_file {
        return Err("--log-file and --trace-file must be different files".to_string());
    }

    Ok(Options { level, log_file, trace, trace_file })
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, String> {
    match args.iter().position(|arg| arg == flag) {
        Some(idx) => match args.get(idx + 1) {
            Some(value) => Ok(Some(value.as_str())),
            None => Err(format!("{flag} requires a value")),
        },
        None => Ok(None),
    }
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}

fn install(opts: &Options) -> Result<(), String> {
    // Log layer: only RhetSim's own messages. Third-party crates (Cranelift,
    // for example) write their own logs at INFO/DEBUG and must not pollute
    // the simulator log. Trace targets stay OFF even at --log-level trace so
    // the two channels never overlap.
    let mut log_targets = Targets::new().with_default(LevelFilter::OFF);
    for &target in LOG_TARGETS {
        log_targets = log_targets.with_target(target, level_filter(opts.level));
    }
    for category in TraceCategory::ALL {
        log_targets = log_targets.with_target(category.target(), LevelFilter::OFF);
    }

    // Trace layer: only the requested categories.
    let mut trace_targets = Targets::new();
    for category in &opts.trace {
        trace_targets = trace_targets.with_target(category.target(), LevelFilter::TRACE);
    }

    let log_writer = make_writer(opts.log_file.as_deref())?;
    let log_ansi = opts.log_file.is_none() && io::stderr().is_terminal();

    let trace_writer = make_writer(opts.trace_file.as_deref())?;

    let log_layer = fmt::layer()
        .with_writer(log_writer)
        .with_ansi(log_ansi)
        .with_filter(log_targets);

    // Trace lines are machine-readable: no level/target noise and no colors.
    let trace_layer = fmt::layer()
        .with_writer(trace_writer)
        .with_ansi(false)
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_filter(trace_targets);

    tracing_subscriber::registry()
        .with(log_layer)
        .with(trace_layer)
        .try_init()
        .map_err(|e| format!("failed to install logger: {e}"))
}

fn make_writer(path: Option<&str>) -> Result<BoxMakeWriter, String> {
    match path {
        Some(path) => {
            let file = File::create(path)
                .map_err(|e| format!("failed to create '{path}': {e}"))?;
            Ok(BoxMakeWriter::new(Arc::new(file)))
        }
        None => Ok(BoxMakeWriter::new(io::stderr)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_defaults() {
        let opts = parse(&args(&["platform.toml"])).unwrap();
        assert_eq!(opts.level, LogLevel::Info);
        assert!(opts.trace.is_empty());
        assert!(opts.log_file.is_none());
        assert!(opts.trace_file.is_none());
    }

    #[test]
    fn parses_diagnostic_flags() {
        let opts = parse(&args(&[
            "platform.toml",
            "--log-level",
            "debug",
            "--trace",
            "exec,cpu",
            "--trace-file",
            "trace.log",
        ]))
        .unwrap();
        assert_eq!(opts.level, LogLevel::Debug);
        assert_eq!(opts.trace, vec![TraceCategory::Exec, TraceCategory::Cpu]);
        assert_eq!(opts.trace_file.as_deref(), Some("trace.log"));
    }

    #[test]
    fn rejects_missing_or_bad_values() {
        assert!(parse(&args(&["platform.toml", "--trace"])).is_err());
        assert!(parse(&args(&["platform.toml", "--log-level", "loud"])).is_err());
        assert!(parse(&args(&["platform.toml", "--trace", "nope"])).is_err());
        assert!(parse(&args(&[
            "platform.toml",
            "--log-file",
            "same.log",
            "--trace-file",
            "same.log",
        ]))
        .is_err());
    }
}
