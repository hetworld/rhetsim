//! Boot Linux kernel via OpenSBI on the RISC-V CPU simulator.
//!
//! Layout (matches QEMU virt machine):
//!   CLINT  @ 0x0200_0000
//!   UART   @ 0x1000_0000
//!   DRAM   @ 0x8000_0000  (64 MiB)
//!   OpenSBI loaded at 0x8000_0000
//!   Linux  loaded at 0x8020_0000  (fw_jump payload address)
//!   DTB    loaded at 0x8200_0000

use std::sync::Arc;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemTarget};
use rhetsim_cpu_riscv::cpu::RiscvCpu;
use rhetsim_cpu_riscv::csr;

const DRAM_BASE: u64  = 0x8000_0000;
const DRAM_SIZE: usize = 64 * 1024 * 1024;
const CLINT_BASE: u64  = 0x0200_0000;
const UART_BASE: u64   = 0x1000_0000;
const KERNEL_ADDR: u64 = 0x8020_0000;
const DTB_ADDR: u64    = 0x8200_0000;

#[test]
fn linux_boot() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap();
    let fw_path = workspace.parent().unwrap()
        .join("opensbi/build/platform/generic/firmware/fw_jump.bin");
    let kernel_path = workspace.parent().unwrap()
        .join("linux-6.1/arch/riscv/boot/Image");
    let dtb_path = workspace.join("platforms/reference/server/riscv-virt.dtb");

    let fw_data = match std::fs::read(&fw_path) {
        Ok(d) => d,
        Err(e) => { eprintln!("Skipping: {}", e); return; }
    };
    let kernel_data = match std::fs::read(&kernel_path) {
        Ok(d) => d,
        Err(e) => { eprintln!("Skipping: {}", e); return; }
    };
    let dtb_data = match std::fs::read(&dtb_path) {
        Ok(d) => d,
        Err(e) => { eprintln!("Skipping: {}", e); return; }
    };

    eprintln!("OpenSBI: {} bytes, Kernel: {} bytes, DTB: {} bytes",
        fw_data.len(), kernel_data.len(), dtb_data.len());

    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram0", DRAM_SIZE, DRAM_BASE));
    let clint = Arc::new(rhetsim_intc_clint::Clint::new("clint0", 1, CLINT_BASE));
    let uart = Arc::new(rhetsim_io_uart::Uart::new("uart0"));

    let mut bus = MemBus::new("sys-bus");
    bus.map(dram.clone() as Arc<dyn MemTarget>, DRAM_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(clint.clone() as Arc<dyn MemTarget>, CLINT_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(uart.clone() as Arc<dyn MemTarget>, UART_BASE, MemAttrs::rw(), None).unwrap();
    let bus = Arc::new(bus);

    let mut cpu = RiscvCpu::with_bus("rv-core0", bus, DRAM_BASE);
    cpu.load_binary(&fw_data, 0);
    cpu.load_binary(&kernel_data, KERNEL_ADDR - DRAM_BASE);
    cpu.load_binary(&dtb_data, DTB_ADDR - DRAM_BASE);

    let (dram_ptr, dram_base, dram_sz) = dram.host_info();
    cpu.set_dram_direct(dram_ptr, dram_base, dram_sz);

    cpu.set_reg(10, 0);
    cpu.set_reg(11, DTB_ADDR);
    cpu.set_pc(DRAM_BASE);

    let max_steps: u64 = 100_000_000;
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

        // Detect idle: M-mode with no new UART output for 1M steps
        // Only after kernel has produced output (skip OpenSBI init phase)
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

    assert!(output_str.contains("Run /init as init process"),
        "Linux kernel should boot and run /init (got {} bytes in {} steps)",
        all_output.len(), total_steps);
}
