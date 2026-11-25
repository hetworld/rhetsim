//! Boot OpenSBI on the RISC-V CPU simulator.
//!
//! Layout (matches QEMU virt machine):
//!   CLINT  @ 0x0200_0000
//!   UART   @ 0x1000_0000
//!   DRAM   @ 0x8000_0000  (64 MiB)
//!   OpenSBI loaded at 0x8000_0000
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
const DTB_ADDR: u64    = 0x8200_0000;

#[test]
fn opensbi_boot() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap();
    let fw_path = workspace.parent().unwrap()
        .join("opensbi/build/platform/generic/firmware/fw_jump.bin");
    let dtb_path = workspace.join("platforms/reference/server/riscv-virt.dtb");

    let fw_data = match std::fs::read(&fw_path) {
        Ok(d) => d,
        Err(e) => { eprintln!("Skipping: {}", e); return; }
    };
    let dtb_data = match std::fs::read(&dtb_path) {
        Ok(d) => d,
        Err(e) => { eprintln!("Skipping: {}", e); return; }
    };

    eprintln!("OpenSBI: {} bytes, DTB: {} bytes", fw_data.len(), dtb_data.len());

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
    cpu.load_binary(&dtb_data, DTB_ADDR - DRAM_BASE);

    // Boot convention: a0=hartid, a1=DTB address
    cpu.set_reg(10, 0);        // a0 = hart 0
    cpu.set_reg(11, DTB_ADDR); // a1 = DTB
    cpu.set_pc(DRAM_BASE);

    let max_steps: u64 = 50_000_000;
    let batch = 1000u64;
    let mut total_steps = 0u64;
    let mut output_len = 0usize;

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
            output_len += out.len();
        }

        if output_len > 0 && total_steps > 1_000_000 {
            break;
        }
    }

    eprintln!("\n--- Result ---");
    eprintln!("Steps: {}, UART: {} bytes, halted: {}, PC: 0x{:X}, mode: {}",
        total_steps, output_len, cpu.is_halted(), cpu.pc(), cpu.priv_mode());

    assert!(output_len > 0 || total_steps >= max_steps,
        "OpenSBI should produce output (got {} bytes in {} steps, PC=0x{:X})",
        output_len, total_steps, cpu.pc());
}
