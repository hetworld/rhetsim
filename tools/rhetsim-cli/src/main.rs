//! rhetsim-cli — load manifest, instantiate platform, run simulation

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps};
use rhetsim_manifest::manifest::{Manifest, UnitDesc, BusDesc};
use rhetsim_runtime::scheduler::Scheduler;
use rhetsim_cpu_riscv::csr;

fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: rhetsim-cli <manifest.toml> [options]");
        eprintln!("Options:");
        eprintln!("  --time <ns>     Simulation duration in nanoseconds (default: 10000)");
        eprintln!("  --quantum <ns>  Scheduler quantum in nanoseconds (default: 1000000)");
        eprintln!("  --steps <n>     Max instruction steps for boot mode (default: 100000000)");
        eprintln!("  --firmware <path>  Override firmware binary path");
        eprintln!("  --kernel <path>    Override kernel image path");
        eprintln!("  --dtb <path>       Override device tree blob path");
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
            eprintln!("Error loading manifest: {e}");
            std::process::exit(1);
        }
    };

    println!("Platform: {}", manifest.name);
    println!("Units:    {}", manifest.units.len());
    println!("Buses:    {}", manifest.buses.len());

    if manifest.boot.firmware.is_some() {
        let max_steps = parse_opt(&args, "--steps", 100_000_000u64);
        boot_riscv_linux(&manifest, manifest_dir, &args, max_steps);
        return;
    }

    let run_time = parse_opt(&args, "--time", 10_000u64);
    let quantum = parse_opt(&args, "--quantum", 1_000_000u64);

    let exec_units = match build_platform(&manifest) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error building platform: {e}");
            std::process::exit(1);
        }
    };

    if exec_units.is_empty() {
        println!("No executable units found. Nothing to simulate.");
        return;
    }

    let mut scheduler = Scheduler::new(quantum);
    for (id, kind, unit_arc) in &exec_units {
        scheduler.add_unit(Arc::clone(unit_arc));
        println!("  [exec] {id} ({kind})");
    }

    println!(
        "Running simulation for {} ns (quantum={}ns, {} executable units)",
        run_time,
        quantum,
        exec_units.len()
    );

    match scheduler.run_default(run_time) {
        Ok(stats) => {
            println!("Simulation complete:");
            println!("  Quanta executed: {}", stats.quanta_executed);
            println!("  Total ns elapsed: {}", stats.total_ns_elapsed);
        }
        Err(e) => {
            eprintln!("Simulation error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Boot path: firmware + kernel direct execution ─────────────

fn boot_riscv_linux(manifest: &Manifest, manifest_dir: &Path, args: &[String], max_steps: u64) {
    let boot = &manifest.boot;

    let fw_path = resolve_path(args, "--firmware", boot.firmware.as_deref(), manifest_dir);
    let kernel_path = resolve_path(args, "--kernel", boot.kernel.as_deref(), manifest_dir);
    let dtb_path = resolve_path(args, "--dtb", boot.dtb.as_deref(), manifest_dir);

    let fw_data = read_required(&fw_path, "firmware");
    let kernel_data = read_required(&kernel_path, "kernel");
    let dtb_data = read_required(&dtb_path, "DTB");

    let dram_desc = manifest.units.iter().find(|u| u.kind == "rhetsim-mem-dram")
        .expect("boot requires a rhetsim-mem-dram unit");
    let clint_desc = manifest.units.iter().find(|u| u.kind == "rhetsim-intc-clint")
        .expect("boot requires a rhetsim-intc-clint unit");
    let cpu_desc = manifest.units.iter().find(|u| u.kind == "rhetsim-cpu-riscv")
        .expect("boot requires a rhetsim-cpu-riscv unit");

    let dram_size = cfg_usize(&dram_desc.config, "size",
        cfg_usize(&dram_desc.config, "size_mb", 64) * 1024 * 1024);
    let dram_base = cfg_u64(&dram_desc.config, "base", 0x8000_0000);
    let clint_harts = cfg_usize(&clint_desc.config, "num_harts", 1);
    let clint_base = cfg_u64(&clint_desc.config, "base", 0x0200_0000);
    let mem_base = cfg_u64(&cpu_desc.config, "mem_base", dram_base);

    let kernel_addr = dram_base + 0x0020_0000;
    let dtb_addr = dram_base + 0x0200_0000;

    eprintln!("Boot layout:");
    eprintln!("  DRAM    @ 0x{:08X}  ({} MiB)", dram_base, dram_size / (1024 * 1024));
    eprintln!("  CLINT   @ 0x{:08X}", clint_base);
    eprintln!("  Firmware  {:>8} bytes -> 0x{:08X}", fw_data.len(), dram_base);
    eprintln!("  Kernel    {:>8} bytes -> 0x{:08X}", kernel_data.len(), kernel_addr);
    eprintln!("  DTB       {:>8} bytes -> 0x{:08X}", dtb_data.len(), dtb_addr);

    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base(&dram_desc.id, dram_size, dram_base));
    let clint = Arc::new(rhetsim_intc_clint::Clint::new(&clint_desc.id, clint_harts, clint_base));
    let uart = Arc::new(rhetsim_io_uart::Uart::new("uart0"));

    let uart_base = find_target_base(manifest, "uart0").unwrap_or(0x1000_0000);

    let mut bus = MemBus::new("sys-bus");
    bus.map(dram.clone() as Arc<dyn MemTarget>, dram_base, MemAttrs::rw(), None).unwrap();
    bus.map(clint.clone() as Arc<dyn MemTarget>, clint_base, MemAttrs::rw(), None).unwrap();
    bus.map(uart.clone() as Arc<dyn MemTarget>, uart_base, MemAttrs::rw(), None).unwrap();
    let bus = Arc::new(bus);

    let mut cpu = rhetsim_cpu_riscv::RiscvCpu::with_bus(&cpu_desc.id, bus, mem_base);
    cpu.load_binary(&fw_data, 0);
    cpu.load_binary(&kernel_data, kernel_addr - dram_base);
    cpu.load_binary(&dtb_data, dtb_addr - dram_base);

    let (dram_ptr, dram_host_base, dram_sz) = dram.host_info();
    cpu.set_dram_direct(dram_ptr, dram_host_base, dram_sz);

    cpu.set_reg(10, 0);
    cpu.set_reg(11, dtb_addr);
    cpu.set_pc(dram_base);

    eprintln!("Booting... (max {} steps)", max_steps);

    let batch = 10_000u64;
    let mut total_steps = 0u64;
    let mut all_output = Vec::new();
    let mut idle_batches = 0u32;
    let mut last_uart_len = 0usize;

    while !cpu.is_halted() && total_steps < max_steps {
        clint.tick(batch);
        if clint.timer_pending(0) {
            cpu.csr_mut().mip |= csr::MIP_MTIP;
        } else {
            cpu.csr_mut().mip &= !csr::MIP_MTIP;
        }
        if clint.sw_pending(0) {
            cpu.csr_mut().mip |= csr::MIP_MSIP;
        } else {
            cpu.csr_mut().mip &= !csr::MIP_MSIP;
        }

        cpu.exec(batch).unwrap();
        total_steps += batch;

        let out = uart.take_output();
        if !out.is_empty() {
            all_output.extend_from_slice(&out);
        }

        if total_steps % 10_000_000 == 0 {
            eprintln!("[{}M steps] PC=0x{:X} mode={} uart={}B",
                total_steps / 1_000_000, cpu.pc(), cpu.priv_mode(), all_output.len());
        }

        if all_output.len() > 1000 && cpu.priv_mode() == csr::PRV_M && all_output.len() == last_uart_len {
            idle_batches += 1;
            if idle_batches > 100 {
                eprintln!("CPU idle in M-mode for {}K steps, stopping.", idle_batches * 10);
                break;
            }
        } else {
            idle_batches = 0;
        }
        last_uart_len = all_output.len();
    }

    let output_str = String::from_utf8_lossy(&all_output);
    eprintln!("\n=== UART Output ({} bytes) ===\n{}", all_output.len(), output_str);
    eprintln!("\n--- Result ---");
    eprintln!("Steps: {}, halted: {}, PC: 0x{:X}, mode: {}",
        total_steps, cpu.is_halted(), cpu.pc(), cpu.priv_mode());

    if output_str.contains("Run /init as init process") {
        eprintln!("Boot SUCCESS: Linux reached /init");
    } else {
        eprintln!("Boot INCOMPLETE: did not reach /init ({} bytes output in {} steps)",
            all_output.len(), total_steps);
        std::process::exit(1);
    }
}

fn resolve_path(args: &[String], flag: &str, manifest_val: Option<&str>, manifest_dir: &Path) -> String {
    if let Some(cli_val) = parse_str_opt(args, flag) {
        return cli_val;
    }
    let name = manifest_val.unwrap_or_else(|| {
        eprintln!("Missing {flag} in manifest [boot] section and no {flag} CLI override");
        std::process::exit(1);
    });
    let p = manifest_dir.join(name);
    p.to_string_lossy().into_owned()
}

fn read_required(path: &str, label: &str) -> Vec<u8> {
    match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to read {label} at '{path}': {e}");
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
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
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

fn build_platform(
    manifest: &Manifest,
) -> Result<Vec<(String, String, UnitArc)>, String> {
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
                    if let Err(e) = bus.map(
                        Arc::clone(target),
                        target_desc.base,
                        MemAttrs::rw(),
                        None,
                    ) {
                        tracing::warn!("Failed to map {} into {}: {}", target_desc.unit, id, e);
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
            Err(e) => tracing::warn!("Skipping unit '{}': {}", desc.id, e),
        }
    }

    Ok(exec_units)
}

fn is_memory(kind: &str) -> bool {
    matches!(kind, "rhetsim-mem-dram" | "rhetsim-mem-sram" | "rhetsim-mem-gddr")
}

fn create_mem_target(desc: &UnitDesc) -> Result<Option<Arc<dyn MemTarget>>, String> {
    match desc.kind.as_str() {
        "rhetsim-mem-dram" => {
            let size = cfg_usize(&desc.config, "size",
                cfg_usize(&desc.config, "size_mb", 64) * 1024 * 1024);
            let base = cfg_u64(&desc.config, "base", 0);
            Ok(Some(Arc::new(rhetsim_mem_dram::Dram::with_base(&desc.id, size, base))))
        }
        "rhetsim-mem-sram" => {
            let size = cfg_usize(&desc.config, "size", 4096);
            let base = cfg_u64(&desc.config, "base", 0);
            Ok(Some(Arc::new(rhetsim_mem_sram::Sram::with_base(&desc.id, size, base))))
        }
        "rhetsim-mem-gddr" => {
            let size = cfg_usize(&desc.config, "size",
                cfg_usize(&desc.config, "size_mb", 64) * 1024 * 1024);
            let base = cfg_u64(&desc.config, "base", 0);
            Ok(Some(Arc::new(rhetsim_mem_gddr::Gddr::with_base(&desc.id, size, base))))
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
            if let Some(bus) = bus {
                Ok(Some(Box::new(rhetsim_cpu_riscv::RiscvCpu::with_bus(&desc.id, bus, mem_base))))
            } else {
                let mem_size = cfg_usize(&desc.config, "mem_size", 64 * 1024);
                Ok(Some(Box::new(rhetsim_cpu_riscv::RiscvCpu::new(&desc.id, mem_size, mem_base))))
            }
        }
        "rhetsim-cpu-arm" => {
            let mem_base = cfg_u64(&desc.config, "mem_base", 0);
            if let Some(bus) = bus {
                Ok(Some(Box::new(rhetsim_cpu_arm::cpu::ArmCpu::with_bus(&desc.id, bus, mem_base))))
            } else {
                Err(format!("ARM CPU '{}' requires a MemBus (no flat-memory mode)", desc.id))
            }
        }
        "rhetsim-cpu-mock" => {
            Ok(Some(Box::new(rhetsim_cpu_mock::MockCpu::new(&desc.id))))
        }
        "rhetsim-accel-gpgpu" => {
            let vram_size = cfg_usize(&desc.config, "vram_size", 64 * 1024);
            Ok(Some(Box::new(rhetsim_accel_gpgpu::Gpgpu::new(&desc.id, vram_size))))
        }
        "rhetsim-io-uart" => {
            Ok(Some(Box::new(rhetsim_io_uart::Uart::new(&desc.id))))
        }
        "rhetsim-intc-clint" => {
            let num_harts = cfg_usize(&desc.config, "num_harts", 1);
            let base = cfg_u64(&desc.config, "base", 0x0200_0000);
            Ok(Some(Box::new(rhetsim_intc_clint::Clint::new(&desc.id, num_harts, base))))
        }
        "rhetsim-intc-plic" => {
            let num_sources = cfg_usize(&desc.config, "num_sources", 64);
            let num_contexts = cfg_usize(&desc.config, "num_contexts", 2);
            let base = cfg_u64(&desc.config, "base", 0x0C00_0000);
            Ok(Some(Box::new(rhetsim_intc_plic::Plic::new(
                &desc.id, num_sources, num_contexts, base,
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
            tracing::info!("Unknown unit kind '{}', skipping '{}'", desc.kind, desc.id);
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

fn cfg_u64(config: &serde_json::Value, key: &str, default: u64) -> u64 {
    config.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}

fn cfg_usize(config: &serde_json::Value, key: &str, default: usize) -> usize {
    config.get(key).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(default)
}
