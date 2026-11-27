//! rhetsim-cli — load manifest, instantiate platform, run simulation

mod logging;

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::diag::{trace as trace_cat, TraceCategory};
use rhetsim_core::irq::{IrqLine, IrqSink};
use rhetsim_core::mem::{MemAttrs, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps};
use rhetsim_cpu_riscv::{csr, RiscvCpuType, RiscvIrqLine};
use rhetsim_intc_clint::ClintLine;
use rhetsim_intc_plic::PlicContextLine;
use rhetsim_manifest::manifest::{BusDesc, Manifest, UnitDesc};
use rhetsim_runtime::scheduler::Scheduler;
use std::collections::HashMap;
use std::io::{Read as IoRead, Write as IoWrite};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// QEMU virt maps UART0 to PLIC source 10.
const UART_PLIC_IRQ: u32 = 10;
/// Marker printed by the initramfs /init.
const INIT_OK_MARKER: &[u8] = b"RHETSIM_INIT_OK";
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;
use tracing::{debug, error, info, trace, warn};

struct LoadedImage {
    entry: Option<u64>,
}

fn load_image(
    cpu: &mut rhetsim_cpu_riscv::RiscvCpu,
    data: &[u8],
    default_addr: u64,
    dram_base: u64,
    label: &str,
) -> LoadedImage {
    if data.len() >= 4 && &data[..4] == b"\x7fELF" {
        load_elf(cpu, data, dram_base, label)
    } else {
        let offset = default_addr - dram_base;
        cpu.load_binary(data, offset);
        info!(target: "loader", "  {label:<10} {:>8} bytes -> 0x{:08X} (raw binary)", data.len(), default_addr);
        trace!(target: trace_cat::LOADER, label = %label, addr = format_args!("0x{:08X}", default_addr), size = data.len(), "load raw");
        LoadedImage { entry: None }
    }
}

fn load_elf(
    cpu: &mut rhetsim_cpu_riscv::RiscvCpu,
    data: &[u8],
    dram_base: u64,
    label: &str,
) -> LoadedImage {
    let elf = Elf::parse(data).unwrap_or_else(|e| {
        error!(target: "loader", "Failed to parse {label} ELF: {e}");
        std::process::exit(1);
    });
    let mut total_loaded = 0usize;
    let mut lo = u64::MAX;
    let mut hi = 0u64;
    for phdr in &elf.program_headers {
        if phdr.p_type != PT_LOAD || phdr.p_filesz == 0 {
            continue;
        }
        let file_off = phdr.p_offset as usize;
        let file_end = file_off + phdr.p_filesz as usize;
        if file_end > data.len() {
            error!(target: "loader", "{label} ELF: PT_LOAD segment exceeds file size");
            std::process::exit(1);
        }
        let seg_data = &data[file_off..file_end];
        let vaddr = phdr.p_vaddr;
        let offset = vaddr - dram_base;
        cpu.load_binary(seg_data, offset);
        trace!(
            target: trace_cat::LOADER,
            label = %label,
            vaddr = format_args!("0x{:08X}", vaddr),
            filesz = phdr.p_filesz,
            memsz = phdr.p_memsz,
            "load segment"
        );
        total_loaded += seg_data.len();
        lo = lo.min(vaddr);
        hi = hi.max(vaddr + phdr.p_memsz);
    }
    info!(target: "loader", "  {label:<10} {:>8} bytes -> 0x{:08X}..0x{:08X} (ELF, entry 0x{:X})",
        total_loaded, lo, hi, elf.entry);
    LoadedImage {
        entry: Some(elf.entry),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let diag = match logging::init(&args) {
        Ok(diag) => diag,
        Err(e) => {
            eprintln!("rhetsim-cli: {e}");
            std::process::exit(2);
        }
    };

    if args.len() < 2 {
        eprintln!("Usage: rhetsim-cli <manifest.toml> [options]");
        eprintln!("Options:");
        eprintln!("  --time <ns>     Simulation duration in nanoseconds (default: 10000)");
        eprintln!("  --quantum <ns>  Scheduler quantum in nanoseconds (default: 1000000)");
        eprintln!("  --steps <n>     Max instruction steps for boot mode (default: 100000000)");
        eprintln!("  --firmware <path>  Override firmware binary path");
        eprintln!("  --kernel <path>    Override kernel image path");
        eprintln!("  --initrd <path>    Override initrd/initramfs path");
        eprintln!("  --initrd-addr <addr>  Override initrd load address");
        eprintln!("  --dtb <path>       Override device tree blob path");
        eprintln!("  --batch         Non-interactive mode (UART output goes to stdout)");
        eprintln!("  --uart-log <path>  Write UART output to file instead of stdout (batch mode)");
        eprintln!("  --log-level <level>  Simulator log level: error|warn|info|debug|trace (default: info)");
        eprintln!("  --log-file <path>    Write simulator logs to file (default: stderr)");
        eprintln!("  --trace <items>      Execution trace categories: exec,cpu,jit,int,plic,clint,uart,loader,all");
        eprintln!("  --trace-file <path>  Write execution trace to file (default: stderr)");
        eprintln!(
            "  --no-jit             Disable the JIT (interpreter only; useful for debugging)"
        );
        eprintln!("  --list-units    List available unit kinds and exit");
        std::process::exit(1);
    }

    if args.iter().any(|a| a == "--list-units") {
        println!("Available unit kinds:");
        for kind in KNOWN_KINDS {
            println!("  {kind}");
        }
        return;
    }

    let manifest_path = Path::new(&args[1]);
    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));

    let manifest = match rhetsim_manifest::loader::load_from_file(manifest_path) {
        Ok(m) => m,
        Err(e) => {
            error!(target: "cli", "Error loading manifest: {e}");
            std::process::exit(1);
        }
    };

    info!(target: "platform", "Platform: {}", manifest.name);
    info!(target: "platform", "Units:    {}", manifest.units.len());
    info!(target: "platform", "Buses:    {}", manifest.buses.len());

    let has_boot = manifest.boot.firmware.is_some() || parse_str_opt(&args, "--firmware").is_some();
    if has_boot {
        let interactive = !args.iter().any(|a| a == "--batch");
        let max_steps = if interactive {
            u64::MAX
        } else {
            parse_opt(&args, "--steps", 100_000_000u64)
        };
        boot_riscv_linux(
            &manifest,
            manifest_dir,
            &args,
            max_steps,
            interactive,
            &diag,
        );
        return;
    }

    let run_time = parse_opt(&args, "--time", 10_000u64);
    let quantum = parse_opt(&args, "--quantum", 1_000_000u64);

    let exec_units = match build_platform(&manifest) {
        Ok(r) => r,
        Err(e) => {
            error!(target: "cli", "Error building platform: {e}");
            std::process::exit(1);
        }
    };

    if exec_units.is_empty() {
        info!(target: "sched", "No executable units found. Nothing to simulate.");
        return;
    }

    let mut scheduler = Scheduler::new(quantum);
    for (id, kind, unit_arc) in &exec_units {
        scheduler.add_unit(Arc::clone(unit_arc));
        info!(target: "sched", "  [exec] {id} ({kind})");
    }

    info!(
        target: "sched",
        "Running simulation for {} ns (quantum={}ns, {} executable units)",
        run_time,
        quantum,
        exec_units.len()
    );

    match scheduler.run_default(run_time) {
        Ok(stats) => {
            info!(target: "sched", "Simulation complete:");
            info!(target: "sched", "  Quanta executed: {}", stats.quanta_executed);
            info!(target: "sched", "  Total ns elapsed: {}", stats.total_ns_elapsed);
        }
        Err(e) => {
            error!(target: "sched", "Simulation error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Boot path: firmware + kernel direct execution ─────────────

fn boot_riscv_linux(
    manifest: &Manifest,
    manifest_dir: &Path,
    args: &[String],
    max_steps: u64,
    interactive: bool,
    diag: &logging::Options,
) {
    let boot = &manifest.boot;

    let fw_path = resolve_path(args, "--firmware", boot.firmware.as_deref(), manifest_dir);
    let kernel_path = resolve_path(args, "--kernel", boot.kernel.as_deref(), manifest_dir);
    let initrd_path = resolve_opt_path(args, "--initrd", boot.initrd.as_deref(), manifest_dir);
    let dtb_path = resolve_path(args, "--dtb", boot.dtb.as_deref(), manifest_dir);

    let fw_data = read_required(&fw_path, "firmware");
    let kernel_data = read_required(&kernel_path, "kernel");
    let mut dtb_data = read_required(&dtb_path, "DTB");
    let initrd_data = initrd_path.as_deref().map(|p| read_required(p, "initrd"));

    let dram_desc = manifest
        .units
        .iter()
        .find(|u| u.kind == "rhetsim-mem-dram")
        .expect("boot requires a rhetsim-mem-dram unit");
    let clint_desc = manifest
        .units
        .iter()
        .find(|u| u.kind == "rhetsim-intc-clint")
        .expect("boot requires a rhetsim-intc-clint unit");
    let plic_desc = manifest
        .units
        .iter()
        .find(|u| u.kind == "rhetsim-intc-plic")
        .expect("boot requires a rhetsim-intc-plic unit");
    let cpu_descs: Vec<&UnitDesc> = manifest
        .units
        .iter()
        .filter(|u| u.kind == "rhetsim-cpu-riscv")
        .collect();
    if cpu_descs.is_empty() {
        error!(target: "boot", "boot requires at least one rhetsim-cpu-riscv unit");
        std::process::exit(1);
    }
    let num_harts = cpu_descs.len();

    let dram_size = cfg_usize(
        &dram_desc.config,
        "size",
        cfg_usize(&dram_desc.config, "size_mb", 64) * 1024 * 1024,
    );
    let dram_base = cfg_u64(&dram_desc.config, "base", 0x8000_0000);
    let clint_harts = cfg_usize(&clint_desc.config, "num_harts", num_harts).max(num_harts);
    let clint_base = cfg_u64(&clint_desc.config, "base", 0x0200_0000);
    let plic_base = cfg_u64(&plic_desc.config, "base", 0x0c00_0000);
    let plic_sources = cfg_usize(&plic_desc.config, "num_sources", 64);
    let plic_contexts =
        cfg_usize(&plic_desc.config, "num_contexts", num_harts * 2).max(num_harts * 2);

    let fw_addr = boot.firmware_addr.unwrap_or(dram_base);
    let kernel_addr = boot.kernel_addr.unwrap_or(dram_base + 0x0020_0000);
    let dtb_addr = boot.dtb_addr.unwrap_or(dram_base + 0x0200_0000);
    let initrd_addr = parse_opt(
        args,
        "--initrd-addr",
        boot.initrd_addr.unwrap_or(dram_base + 0x0300_0000),
    );

    if let Some(data) = initrd_data.as_ref() {
        let initrd_end = match initrd_addr.checked_add(data.len() as u64) {
            Some(v) => v,
            None => {
                error!(target: "boot", "initrd address overflow");
                std::process::exit(1);
            }
        };
        if initrd_addr < dram_base || initrd_end > dram_base + dram_size as u64 {
            error!(target: "boot",
                "initrd 0x{:08X}..0x{:08X} does not fit DRAM 0x{:08X}..0x{:08X}",
                initrd_addr, initrd_end, dram_base, dram_base + dram_size as u64);
            std::process::exit(1);
        }
        if let Err(e) = patch_dtb_initrd(&mut dtb_data, initrd_addr, initrd_end) {
            error!(target: "boot", "failed to patch DTB for initrd: {e}");
            std::process::exit(1);
        }
        info!(target: "boot", "  Initrd  @ 0x{:08X}  ({} bytes)", initrd_addr, data.len());
    }

    info!(target: "boot", "Boot layout:");
    info!(target: "boot", "  DRAM    @ 0x{:08X}  ({} MiB)", dram_base, dram_size / (1024 * 1024));
    info!(target: "boot", "  CLINT   @ 0x{:08X}  ({} harts)", clint_base, clint_harts);
    info!(target: "boot", "  PLIC    @ 0x{:08X}  ({} sources, {} contexts)", plic_base, plic_sources, plic_contexts);
    info!(target: "boot", "  HARTs   {}", num_harts);

    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base(
        &dram_desc.id,
        dram_size,
        dram_base,
    ));
    let clint = Arc::new(rhetsim_intc_clint::Clint::new(
        &clint_desc.id,
        clint_harts,
        clint_base,
    ));
    let plic = Arc::new(rhetsim_intc_plic::Plic::new(
        &plic_desc.id,
        plic_sources,
        plic_contexts,
        plic_base,
    ));
    let uart = Arc::new(rhetsim_io_uart::Uart::new("uart0"));

    let uart_base = find_target_base(manifest, "uart0").unwrap_or(0x1000_0000);

    let mut bus = MemBus::new("sys-bus");
    bus.map(
        dram.clone() as Arc<dyn MemTarget>,
        dram_base,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        clint.clone() as Arc<dyn MemTarget>,
        clint_base,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        plic.clone() as Arc<dyn MemTarget>,
        plic_base,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        uart.clone() as Arc<dyn MemTarget>,
        uart_base,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus = Arc::new(bus);

    // UART IRQ 10 is a PLIC source; per hart, PLIC context 2*hart is the
    // M-mode output (MEIP) and 2*hart+1 is the S-mode output (SEIP).
    uart.set_irq_line(IrqLine::new(
        plic.clone() as Arc<dyn IrqSink>,
        UART_PLIC_IRQ,
    ));

    let mut cpus: Vec<rhetsim_cpu_riscv::RiscvCpu> = Vec::with_capacity(num_harts);
    for (hart, desc) in cpu_descs.iter().enumerate() {
        let hart_mem_base = cfg_u64(&desc.config, "mem_base", dram_base);
        let cpu_type = match cfg_riscv_cpu_type(&desc.config) {
            Ok(cpu_type) => cpu_type,
            Err(err) => {
                error!(target: "boot", "{}", err);
                std::process::exit(1);
            }
        };
        let mut cpu = rhetsim_cpu_riscv::RiscvCpu::with_bus_type(
            &desc.id,
            bus.clone(),
            hart_mem_base,
            cpu_type,
        );
        cpu.set_hartid(hart as u64);
        cpu.set_irq_line(
            RiscvIrqLine::MTimer,
            Arc::new(ClintLine::new(clint.clone(), hart, true)),
        );
        cpu.set_irq_line(
            RiscvIrqLine::MSoft,
            Arc::new(ClintLine::new(clint.clone(), hart, false)),
        );
        cpu.set_irq_line(
            RiscvIrqLine::MExt,
            Arc::new(PlicContextLine::new(plic.clone(), hart * 2)),
        );
        cpu.set_irq_line(
            RiscvIrqLine::SExt,
            Arc::new(PlicContextLine::new(plic.clone(), hart * 2 + 1)),
        );
        cpus.push(cpu);
    }

    // Per-instruction trace must not run through cached JIT basic blocks.
    if diag.trace_enabled(TraceCategory::Exec) || diag.trace_enabled(TraceCategory::Cpu) {
        info!(target: "boot", "instruction trace enabled; JIT disabled");
        for cpu in &mut cpus {
            cpu.set_jit_enabled(false);
        }
    }
    if args.iter().any(|a| a == "--no-jit") {
        info!(target: "boot", "JIT disabled by --no-jit");
        for cpu in &mut cpus {
            cpu.set_jit_enabled(false);
        }
    }

    // Firmware, kernel and DTB are shared in DRAM; load them through hart 0.
    let fw_result = load_image(&mut cpus[0], &fw_data, fw_addr, dram_base, "Firmware");
    let _kernel_result = load_image(&mut cpus[0], &kernel_data, kernel_addr, dram_base, "Kernel");
    cpus[0].load_binary(&dtb_data, dtb_addr - dram_base);
    info!(target: "loader", "  DTB       {:>8} bytes -> 0x{:08X} (raw)", dtb_data.len(), dtb_addr);
    trace!(target: trace_cat::LOADER, addr = format_args!("0x{:08X}", dtb_addr), size = dtb_data.len(), "load dtb raw");

    if let Some(data) = initrd_data.as_ref() {
        cpus[0].load_binary(data, initrd_addr - dram_base);
        info!(target: "loader", "  Initrd    {:>8} bytes -> 0x{:08X} (raw)", data.len(), initrd_addr);
        trace!(target: trace_cat::LOADER, addr = format_args!("0x{:08X}", initrd_addr), size = data.len(), "load initrd raw");
    }

    let entry_pc = fw_result.entry.unwrap_or(fw_addr);

    let (dram_ptr, dram_host_base, dram_sz) = dram.host_info();
    for (hart, cpu) in cpus.iter_mut().enumerate() {
        cpu.set_dram_direct(dram_ptr, dram_host_base, dram_sz);
        cpu.set_reg(10, hart as u64); // a0 = hartid
        cpu.set_reg(11, dtb_addr); // a1 = DTB
        cpu.set_pc(entry_pc);
    }
    info!(target: "boot", "  Entry PC  0x{:08X} ({} harts)", entry_pc, num_harts);

    let _raw_guard = if interactive {
        info!(target: "boot", "Booting interactive (Ctrl-A X to quit)...");
        uart.set_console(true);
        Some(RawTermGuard::enter())
    } else {
        info!(target: "boot", "Booting... (max {} steps)", max_steps);
        None
    };

    // Guest console: a data path, separate from simulator logs. In batch mode
    // stream UART bytes as they are produced (stdout or --uart-log file);
    // interactive mode writes them straight to the terminal from the device.
    let uart_log_path = parse_str_opt(args, "--uart-log");
    let mut console_out: Option<Box<dyn IoWrite>> = if interactive {
        None
    } else if let Some(path) = uart_log_path.as_deref() {
        match std::fs::File::create(path) {
            Ok(file) => Some(Box::new(file)),
            Err(e) => {
                error!(target: "console", "Failed to create UART log '{path}': {e}");
                std::process::exit(1);
            }
        }
    } else {
        Some(Box::new(std::io::stdout()))
    };

    let stop = Arc::new(AtomicBool::new(false));
    let stdin_thread = if interactive {
        let uart_ref = uart.clone();
        let stop_ref = stop.clone();
        Some(std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 64];
            let mut ctrl_a = false;
            loop {
                if stop_ref.load(Ordering::Relaxed) {
                    break;
                }
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for &b in &buf[..n] {
                            if ctrl_a {
                                ctrl_a = false;
                                if b == b'x' || b == b'X' {
                                    stop_ref.store(true, Ordering::Relaxed);
                                    return;
                                }
                                uart_ref.push_input(&[0x01, b]);
                                continue;
                            }
                            if b == 0x01 {
                                ctrl_a = true;
                                continue;
                            }
                            uart_ref.push_input(&[b]);
                        }
                    }
                    Err(_) => break,
                }
            }
        }))
    } else {
        None
    };

    let batch = 10_000u64;
    let mut total_steps = 0u64;
    let mut all_output = Vec::new();
    let mut idle_batches = 0u32;
    let mut last_uart_len = 0usize;

    while cpus.iter().any(|c| !c.is_halted())
        && total_steps < max_steps
        && !stop.load(Ordering::Relaxed)
    {
        clint.tick(batch);

        for cpu in cpus.iter_mut() {
            cpu.exec(batch).unwrap();
        }
        total_steps += batch * num_harts as u64;

        if !interactive {
            let out = uart.take_output();
            if !out.is_empty() {
                if let Some(writer) = console_out.as_mut() {
                    writer.write_all(&out).unwrap_or_else(|e| {
                        error!(target: "console", "Failed to write UART output: {e}");
                        std::process::exit(1);
                    });
                    let _ = writer.flush();
                }
                all_output.extend_from_slice(&out);
            }
            if all_output
                .windows(INIT_OK_MARKER.len())
                .any(|w| w == INIT_OK_MARKER)
            {
                info!(target: "boot", "initramfs /init reached; stopping early");
                break;
            }
        }

        if !interactive {
            if total_steps.is_multiple_of(10_000_000) {
                let cpu0 = &cpus[0];
                debug!(target: "boot", steps = total_steps, harts = num_harts,
                    pc = format_args!("0x{:X}", cpu0.pc()),
                    mode = cpu0.priv_mode(), uart_bytes = all_output.len(),
                    mip = format_args!("0x{:X}", cpu0.csr().mip),
                    mie = format_args!("0x{:X}", cpu0.csr().mie),
                    mcause = format_args!("0x{:X}", cpu0.csr().mcause),
                    mtime = clint.mtime(),
                    timer_pending = clint.timer_pending(0),
                    "progress");
                if let Some(cpu1) = cpus.get(1) {
                    debug!(target: "boot", hart1_pc = format_args!("0x{:X}", cpu1.pc()),
                        hart1_mode = cpu1.priv_mode(),
                        hart1_mip = format_args!("0x{:X}", cpu1.csr().mip),
                        "hart1");
                }
            }

            if num_harts == 1
                && all_output.len() > 1000
                && cpus[0].priv_mode() == csr::PRV_M
                && all_output.len() == last_uart_len
            {
                idle_batches += 1;
                if idle_batches > 100 {
                    warn!(target: "boot", "CPU idle in M-mode for {}K steps, stopping.", idle_batches * 10);
                    break;
                }
            } else {
                idle_batches = 0;
            }
            last_uart_len = all_output.len();
        }
    }

    drop(_raw_guard);
    if let Some(t) = stdin_thread {
        let _ = t.join();
    }

    if interactive {
        info!(target: "boot", "Exited ({} steps, {} harts)", total_steps, num_harts);
    } else {
        if let Some(mut writer) = console_out.take() {
            let _ = writer.flush();
        }
        if let Some(path) = uart_log_path.as_deref() {
            info!(target: "console", "UART output written to {} ({} bytes)", path, all_output.len());
        }

        info!(target: "boot", steps = total_steps, harts = num_harts,
            halted = cpus.iter().all(|c| c.is_halted()),
            pc = format_args!("0x{:X}", cpus[0].pc()), mode = cpus[0].priv_mode(),
            "simulation finished");

        let output_str = String::from_utf8_lossy(&all_output);
        if output_str.contains("Run /init as init process") {
            info!(target: "boot", "Boot SUCCESS: Linux reached /init");
        } else {
            warn!(target: "boot", "Boot INCOMPLETE: did not reach /init ({} bytes output in {} steps)",
                all_output.len(), total_steps);
            std::process::exit(1);
        }
    }
}

fn resolve_opt_path(
    args: &[String],
    flag: &str,
    manifest_val: Option<&str>,
    manifest_dir: &Path,
) -> Option<String> {
    if let Some(cli_val) = parse_str_opt(args, flag) {
        return Some(cli_val);
    }
    manifest_val.map(|name| manifest_dir.join(name).to_string_lossy().into_owned())
}

fn fdt_be32(data: &[u8], off: usize) -> Result<u32, String> {
    data.get(off..off + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "truncated FDT".to_string())
}

fn fdt_cstr(data: &[u8], start: usize, end: usize) -> Result<String, String> {
    if start >= end || end > data.len() {
        return Err("bad FDT string range".to_string());
    }
    let bytes = &data[start..end];
    let len = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| "unterminated FDT string".to_string())?;
    std::str::from_utf8(&bytes[..len])
        .map(|s| s.to_string())
        .map_err(|e| format!("invalid FDT string: {e}"))
}

/// Write the initrd range into /chosen's linux,initrd-start/end properties.
/// The DTB must be compiled from a DTS that already contains both properties.
fn patch_dtb_initrd(dtb: &mut [u8], start: u64, end: u64) -> Result<(), String> {
    const FDT_MAGIC: u32 = 0xd00d_feed;
    if dtb.len() < 40 {
        return Err("DTB too small".to_string());
    }
    if fdt_be32(dtb, 0)? != FDT_MAGIC {
        return Err("bad DTB magic".to_string());
    }
    let off_struct = fdt_be32(dtb, 8)? as usize;
    let off_strings = fdt_be32(dtb, 12)? as usize;
    let size_strings = fdt_be32(dtb, 32)? as usize;
    let size_struct = fdt_be32(dtb, 36)? as usize;
    let end_struct = off_struct
        .checked_add(size_struct)
        .ok_or_else(|| "bad FDT structure range".to_string())?;
    let end_strings = off_strings
        .checked_add(size_strings)
        .ok_or_else(|| "bad FDT strings range".to_string())?;
    if end_struct > dtb.len() || end_strings > dtb.len() {
        return Err("FDT block exceeds file".to_string());
    }

    let mut pos = off_struct;
    let mut depth = 0usize;
    let mut in_chosen = false;
    let mut found_start = false;
    let mut found_end = false;

    while pos + 4 <= end_struct {
        let token = fdt_be32(dtb, pos)?;
        pos += 4;
        match token {
            1 => {
                // FDT_BEGIN_NODE
                let name_start = pos;
                while pos < end_struct && dtb[pos] != 0 {
                    pos += 1;
                }
                if pos >= end_struct {
                    return Err("unterminated FDT node name".to_string());
                }
                let name = std::str::from_utf8(&dtb[name_start..pos]).unwrap_or("");
                pos += 1;
                pos = (pos + 3) & !3;
                depth += 1;
                if depth == 2 && name == "chosen" {
                    in_chosen = true;
                }
            }
            2 => {
                // FDT_END_NODE
                if in_chosen && depth == 2 {
                    break;
                }
                depth = depth.saturating_sub(1);
            }
            3 => {
                // FDT_PROP
                let len = fdt_be32(dtb, pos)? as usize;
                pos += 4;
                let nameoff = fdt_be32(dtb, pos)? as usize;
                pos += 4;
                let value_off = pos;
                if value_off + len > end_struct {
                    return Err("FDT property exceeds structure block".to_string());
                }
                if in_chosen && depth == 2 {
                    let name_start = off_strings
                        .checked_add(nameoff)
                        .ok_or_else(|| "bad FDT property name offset".to_string())?;
                    let name = fdt_cstr(dtb, name_start, end_strings)?;
                    let (is_start, is_end) = match name.as_str() {
                        "linux,initrd-start" => (true, false),
                        "linux,initrd-end" => (false, true),
                        _ => (false, false),
                    };
                    if is_start || is_end {
                        if len != 8 {
                            return Err(format!(
                                "{name} must be 2 address cells (8 bytes), got {len}"
                            ));
                        }
                        let value = if is_start { start } else { end };
                        dtb[value_off..value_off + 8].copy_from_slice(&value.to_be_bytes());
                        if is_start {
                            found_start = true;
                        }
                        if is_end {
                            found_end = true;
                        }
                    }
                }
                pos += (len + 3) & !3;
            }
            4 => {}     // FDT_NOP
            9 => break, // FDT_END
            other => return Err(format!("unknown FDT token 0x{other:x}")),
        }
    }

    if !found_start || !found_end {
        return Err(
            "DTB lacks /chosen linux,initrd-start/end; rebuild it from the updated DTS".to_string(),
        );
    }
    Ok(())
}

fn resolve_path(
    args: &[String],
    flag: &str,
    manifest_val: Option<&str>,
    manifest_dir: &Path,
) -> String {
    if let Some(cli_val) = parse_str_opt(args, flag) {
        return cli_val;
    }
    let name = manifest_val.unwrap_or_else(|| {
        error!(target: "boot", "Missing {flag} in manifest [boot] section and no {flag} CLI override");
        std::process::exit(1);
    });
    let p = manifest_dir.join(name);
    p.to_string_lossy().into_owned()
}

fn read_required(path: &str, label: &str) -> Vec<u8> {
    match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            error!(target: "loader", "Failed to read {label} at '{path}': {e}");
            std::process::exit(1);
        }
    }
}

fn find_target_base(manifest: &Manifest, unit_id: &str) -> Option<u64> {
    for bus_desc in &manifest.buses {
        if let BusDesc::Mem { targets, .. } = bus_desc {
            for t in targets {
                if t.unit == unit_id {
                    return Some(t.base);
                }
            }
        }
    }
    None
}

fn parse_str_opt(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

// ── Generic scheduler path ────────────────────────────────────

fn parse_opt(args: &[String], flag: &str, default: u64) -> u64 {
    args.windows(2)
        .find(|w| w[0] == flag)
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(default)
}

const KNOWN_KINDS: &[&str] = &[
    "rhetsim-cpu-riscv",
    "rhetsim-cpu-arm",
    "rhetsim-cpu-mock",
    "rhetsim-mem-dram",
    "rhetsim-mem-sram",
    "rhetsim-mem-gddr",
    "rhetsim-accel-gpgpu",
    "rhetsim-io-uart",
    "rhetsim-io-dma",
    "rhetsim-io-virtio",
    "rhetsim-intc-clint",
    "rhetsim-intc-plic",
    "rhetsim-intc-gicv3",
    "rhetsim-io-pcie",
];

type UnitArc = Arc<Mutex<Box<dyn Unit>>>;

fn build_platform(manifest: &Manifest) -> Result<Vec<(String, String, UnitArc)>, String> {
    // Phase 1: Create memory targets for bus mapping
    let mut mem_targets: HashMap<String, Arc<dyn MemTarget>> = HashMap::new();
    for desc in &manifest.units {
        if let Some(target) = create_mem_target(desc)? {
            mem_targets.insert(desc.id.clone(), target);
        }
    }

    // Phase 2: Build MemBuses and map targets into them
    let mut buses: HashMap<String, Arc<MemBus>> = HashMap::new();
    for bus_desc in &manifest.buses {
        if let BusDesc::Mem { id, targets, .. } = bus_desc {
            let mut bus = MemBus::new(id);
            for target_desc in targets {
                if let Some(target) = mem_targets.get(&target_desc.unit) {
                    if let Err(e) =
                        bus.map(Arc::clone(target), target_desc.base, MemAttrs::rw(), None)
                    {
                        warn!(target: "platform", "Failed to map {} into {}: {}", target_desc.unit, id, e);
                    }
                }
            }
            buses.insert(id.clone(), Arc::new(bus));
        }
    }

    // Phase 3: Create executable units (CPUs, GPGPUs, peripherals)
    let mut exec_units: Vec<(String, String, UnitArc)> = Vec::new();

    for desc in &manifest.units {
        if is_memory(&desc.kind) {
            continue;
        }
        match create_unit(desc, &buses, &manifest.buses) {
            Ok(Some(unit)) => {
                let unit_ref: &dyn Unit = unit.as_ref();
                let id = unit_ref.id().as_str().to_string();
                let kind = unit_ref.kind().to_string();
                let caps = unit_ref.caps();
                if caps.contains(UnitCaps::EXECUTABLE) {
                    exec_units.push((id, kind, Arc::new(Mutex::new(unit))));
                }
            }
            Ok(None) => {}
            Err(e) => warn!(target: "platform", "Skipping unit '{}': {}", desc.id, e),
        }
    }

    Ok(exec_units)
}

fn is_memory(kind: &str) -> bool {
    matches!(
        kind,
        "rhetsim-mem-dram" | "rhetsim-mem-sram" | "rhetsim-mem-gddr"
    )
}

fn create_mem_target(desc: &UnitDesc) -> Result<Option<Arc<dyn MemTarget>>, String> {
    match desc.kind.as_str() {
        "rhetsim-mem-dram" => {
            let size = cfg_usize(
                &desc.config,
                "size",
                cfg_usize(&desc.config, "size_mb", 64) * 1024 * 1024,
            );
            let base = cfg_u64(&desc.config, "base", 0);
            Ok(Some(Arc::new(rhetsim_mem_dram::Dram::with_base(
                &desc.id, size, base,
            ))))
        }
        "rhetsim-mem-sram" => {
            let size = cfg_usize(&desc.config, "size", 4096);
            let base = cfg_u64(&desc.config, "base", 0);
            Ok(Some(Arc::new(rhetsim_mem_sram::Sram::with_base(
                &desc.id, size, base,
            ))))
        }
        "rhetsim-mem-gddr" => {
            let size = cfg_usize(
                &desc.config,
                "size",
                cfg_usize(&desc.config, "size_mb", 64) * 1024 * 1024,
            );
            let base = cfg_u64(&desc.config, "base", 0);
            Ok(Some(Arc::new(rhetsim_mem_gddr::Gddr::with_base(
                &desc.id, size, base,
            ))))
        }
        _ => Ok(None),
    }
}

fn create_unit(
    desc: &UnitDesc,
    buses: &HashMap<String, Arc<MemBus>>,
    bus_descs: &[BusDesc],
) -> Result<Option<Box<dyn Unit>>, String> {
    let bus = find_bus_for_initiator(&desc.id, buses, bus_descs);

    match desc.kind.as_str() {
        "rhetsim-cpu-riscv" => {
            let mem_base = cfg_u64(&desc.config, "mem_base", 0x8000_0000);
            let cpu_type = cfg_riscv_cpu_type(&desc.config)?;
            if let Some(bus) = bus {
                Ok(Some(Box::new(rhetsim_cpu_riscv::RiscvCpu::with_bus_type(
                    &desc.id, bus, mem_base, cpu_type,
                ))))
            } else {
                let mem_size = cfg_usize(&desc.config, "mem_size", 64 * 1024);
                Ok(Some(Box::new(rhetsim_cpu_riscv::RiscvCpu::new_with_type(
                    &desc.id, mem_size, mem_base, cpu_type,
                ))))
            }
        }
        "rhetsim-cpu-arm" => {
            let mem_base = cfg_u64(&desc.config, "mem_base", 0);
            if let Some(bus) = bus {
                Ok(Some(Box::new(rhetsim_cpu_arm::cpu::ArmCpu::with_bus(
                    &desc.id, bus, mem_base,
                ))))
            } else {
                Err(format!(
                    "ARM CPU '{}' requires a MemBus (no flat-memory mode)",
                    desc.id
                ))
            }
        }
        "rhetsim-cpu-mock" => Ok(Some(Box::new(rhetsim_cpu_mock::MockCpu::new(&desc.id)))),
        "rhetsim-accel-gpgpu" => {
            let vram_size = cfg_usize(&desc.config, "vram_size", 64 * 1024);
            Ok(Some(Box::new(rhetsim_accel_gpgpu::Gpgpu::new(
                &desc.id, vram_size,
            ))))
        }
        "rhetsim-io-uart" => Ok(Some(Box::new(rhetsim_io_uart::Uart::new(&desc.id)))),
        "rhetsim-intc-clint" => {
            let num_harts = cfg_usize(&desc.config, "num_harts", 1);
            let base = cfg_u64(&desc.config, "base", 0x0200_0000);
            Ok(Some(Box::new(rhetsim_intc_clint::Clint::new(
                &desc.id, num_harts, base,
            ))))
        }
        "rhetsim-intc-plic" => {
            let num_sources = cfg_usize(&desc.config, "num_sources", 64);
            let num_contexts = cfg_usize(&desc.config, "num_contexts", 2);
            let base = cfg_u64(&desc.config, "base", 0x0C00_0000);
            Ok(Some(Box::new(rhetsim_intc_plic::Plic::new(
                &desc.id,
                num_sources,
                num_contexts,
                base,
            ))))
        }
        "rhetsim-intc-gicv3" => {
            let num_cpus = cfg_usize(&desc.config, "num_cpus", 1);
            let num_spis = cfg_usize(&desc.config, "num_irqs", 64);
            let base = cfg_u64(&desc.config, "base", 0x0800_0000);
            Ok(Some(Box::new(rhetsim_intc_gicv3::Gicv3::new(
                &desc.id, num_cpus, num_spis, base,
            ))))
        }
        _ => {
            info!(target: "platform", "Unknown unit kind '{}', skipping '{}'", desc.kind, desc.id);
            Ok(None)
        }
    }
}

fn find_bus_for_initiator(
    unit_id: &str,
    buses: &HashMap<String, Arc<MemBus>>,
    bus_descs: &[BusDesc],
) -> Option<Arc<MemBus>> {
    for desc in bus_descs {
        if let BusDesc::Mem { id, initiators, .. } = desc {
            if initiators.iter().any(|i| i == unit_id) {
                return buses.get(id).cloned();
            }
        }
    }
    None
}

fn cfg_riscv_cpu_type(config: &serde_json::Value) -> Result<RiscvCpuType, String> {
    let name = config
        .get("cpu_type")
        .and_then(|v| v.as_str())
        .or_else(|| config.get("isa").and_then(|v| v.as_str()))
        .unwrap_or("rhetsim-rv64imac");
    let cpu_type = RiscvCpuType::parse(name)?;
    cpu_type.ensure_implemented()?;
    Ok(cpu_type)
}

fn cfg_u64(config: &serde_json::Value, key: &str, default: u64) -> u64 {
    config.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}

fn cfg_usize(config: &serde_json::Value, key: &str, default: usize) -> usize {
    config
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
}

// ── Raw terminal mode ─────────────────────────────────────────

struct RawTermGuard {
    orig: libc::termios,
}

impl RawTermGuard {
    fn enter() -> Self {
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            libc::tcgetattr(libc::STDIN_FILENO, &mut orig);
            let mut raw = orig;
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            Self { orig }
        }
    }
}

impl Drop for RawTermGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.orig);
        }
    }
}
