//! rhetsim-cli — load manifest, instantiate platform, run simulation

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps};
use rhetsim_manifest::manifest::{Manifest, UnitDesc, BusDesc};
use rhetsim_runtime::scheduler::Scheduler;

fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: rhetsim-cli <manifest.toml> [--time <ns>] [--quantum <ns>]");
        eprintln!("Options:");
        eprintln!("  --time <ns>     Simulation duration in nanoseconds (default: 10000)");
        eprintln!("  --quantum <ns>  Scheduler quantum in nanoseconds (default: 1000000)");
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

    let manifest_path = &args[1];
    let run_time = parse_opt(&args, "--time", 10_000u64);
    let quantum = parse_opt(&args, "--quantum", 1_000_000u64);

    let manifest = match rhetsim_manifest::loader::load_from_file(Path::new(manifest_path)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Error loading manifest: {e}");
            std::process::exit(1);
        }
    };

    println!("Platform: {}", manifest.name);
    println!("Units:    {}", manifest.units.len());
    println!("Buses:    {}", manifest.buses.len());

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
