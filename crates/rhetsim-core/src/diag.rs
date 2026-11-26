//! Diagnostic output categories shared by the simulator.
//!
//! RhetSim keeps three output channels apart:
//!
//! * guest console (UART) — plain data, never routed through `tracing`;
//! * simulator log — leveled messages (`error`/`warn`/`info`/`debug`/`trace`);
//! * execution trace — opt-in, category-filtered events that can be written
//!   to a separate file.
//!
//! This module defines the stable target names and the category list. The
//! actual subscriber is installed by the host binary (see `rhetsim-cli`).

/// Stable `tracing` targets for execution trace categories.
///
/// Device code should emit events with these targets; the log layer filters
/// the `rhetsim::trace` prefix out, and the trace layer enables only the
/// categories requested on the command line.
pub mod trace {
    pub const EXEC: &str = "rhetsim::trace::exec";
    pub const CPU: &str = "rhetsim::trace::cpu";
    pub const JIT: &str = "rhetsim::trace::jit";
    pub const INT: &str = "rhetsim::trace::int";
    pub const PLIC: &str = "rhetsim::trace::plic";
    pub const CLINT: &str = "rhetsim::trace::clint";
    pub const UART: &str = "rhetsim::trace::uart";
    pub const LOADER: &str = "rhetsim::trace::loader";
}

/// One selectable execution-trace category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceCategory {
    /// Retired instructions: hart, PC, privilege, encoding.
    Exec,
    /// Architectural register state after each traced instruction.
    Cpu,
    /// JIT basic-block execution.
    Jit,
    /// Traps, interrupts and trap returns.
    Int,
    /// PLIC claims, completions and configuration writes.
    Plic,
    /// CLINT `mtimecmp` / MSIP writes.
    Clint,
    /// UART device state and interrupt-line transitions.
    Uart,
    /// Image loading.
    Loader,
}

impl TraceCategory {
    pub const ALL: &'static [TraceCategory] = &[
        Self::Exec,
        Self::Cpu,
        Self::Jit,
        Self::Int,
        Self::Plic,
        Self::Clint,
        Self::Uart,
        Self::Loader,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Exec => "exec",
            Self::Cpu => "cpu",
            Self::Jit => "jit",
            Self::Int => "int",
            Self::Plic => "plic",
            Self::Clint => "clint",
            Self::Uart => "uart",
            Self::Loader => "loader",
        }
    }

    pub const fn target(self) -> &'static str {
        match self {
            Self::Exec => trace::EXEC,
            Self::Cpu => trace::CPU,
            Self::Jit => trace::JIT,
            Self::Int => trace::INT,
            Self::Plic => trace::PLIC,
            Self::Clint => trace::CLINT,
            Self::Uart => trace::UART,
            Self::Loader => trace::LOADER,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// Parse a comma-separated category list such as `exec,cpu,int`.
    /// The special name `all` expands to every category.
    pub fn parse_list(spec: &str) -> Result<Vec<Self>, String> {
        let mut out = Vec::new();
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if item.eq_ignore_ascii_case("all") {
                for c in Self::ALL {
                    if !out.contains(c) {
                        out.push(*c);
                    }
                }
                continue;
            }
            let Some(category) = Self::from_name(item) else {
                let known = Self::ALL
                    .iter()
                    .map(|c| c.name())
                    .collect::<Vec<_>>()
                    .join(",");
                return Err(format!(
                    "unknown trace category '{item}' (known: all,{known})"
                ));
            };
            if !out.contains(&category) {
                out.push(category);
            }
        }
        if out.is_empty() {
            return Err("empty trace category list".to_string());
        }
        Ok(out)
    }
}

/// Simulator log level (`--log-level`). Ordered from least to most verbose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn parse(level: &str) -> Option<Self> {
        match level.to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trace_list() {
        assert_eq!(
            TraceCategory::parse_list("exec, cpu ,int").unwrap(),
            vec![TraceCategory::Exec, TraceCategory::Cpu, TraceCategory::Int]
        );
    }

    #[test]
    fn parse_all_expands_once() {
        let all = TraceCategory::parse_list("all,exec").unwrap();
        assert_eq!(all.len(), TraceCategory::ALL.len());
    }

    #[test]
    fn parse_rejects_unknown_category() {
        assert!(TraceCategory::parse_list("exec,nope").is_err());
        assert!(TraceCategory::parse_list(" , ").is_err());
    }

    #[test]
    fn targets_are_unique() {
        let mut targets: Vec<_> = TraceCategory::ALL.iter().map(|c| c.target()).collect();
        targets.sort_unstable();
        targets.dedup();
        assert_eq!(targets.len(), TraceCategory::ALL.len());
    }

    #[test]
    fn parse_log_level() {
        assert_eq!(LogLevel::parse("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::parse("warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("loud"), None);
    }
}
