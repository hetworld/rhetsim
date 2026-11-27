//! Debug trace for OpenSBI boot — capture the call leading to sbi_hart_hang.

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAccessSize, MemAttrs, MemTarget};
use rhetsim_cpu_riscv::cpu::RiscvCpu;
use rhetsim_cpu_riscv::csr;
use std::sync::Arc;

const DRAM_BASE: u64 = 0x8000_0000;
const DRAM_SIZE: usize = 64 * 1024 * 1024;
const CLINT_BASE: u64 = 0x0200_0000;
const UART_BASE: u64 = 0x1000_0000;
const DTB_ADDR: u64 = 0x8200_0000;
const HANG_ADDR: u64 = 0x800079EC; // sbi_hart_hang

#[test]
fn opensbi_trace() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let fw_path = workspace
        .parent()
        .unwrap()
        .join("opensbi/build/platform/generic/firmware/fw_jump.bin");
    let dtb_path = workspace.join("platforms/reference/server/riscv-virt.dtb");

    let fw_data = match std::fs::read(&fw_path) {
        Ok(d) => d,
        Err(_) => {
            eprintln!("skip");
            return;
        }
    };
    let dtb_data = std::fs::read(&dtb_path).unwrap();

    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base(
        "dram0", DRAM_SIZE, DRAM_BASE,
    ));
    let clint = Arc::new(rhetsim_intc_clint::Clint::new("clint0", 1, CLINT_BASE));
    let uart = Arc::new(rhetsim_io_uart::Uart::new("uart0"));

    let mut bus = MemBus::new("sys-bus");
    bus.map(
        dram.clone() as Arc<dyn MemTarget>,
        DRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        clint.clone() as Arc<dyn MemTarget>,
        CLINT_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        uart.clone() as Arc<dyn MemTarget>,
        UART_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus_arc = Arc::new(bus);

    let mut cpu = RiscvCpu::with_bus("rv-core0", bus_arc.clone(), DRAM_BASE);
    cpu.load_binary(&fw_data, 0);
    cpu.load_binary(&dtb_data, DTB_ADDR - DRAM_BASE);
    cpu.set_reg(10, 0);
    cpu.set_reg(11, DTB_ADDR);
    cpu.set_pc(DRAM_BASE);

    // Ring buffer of last 50 PCs before hang
    let mut pc_ring = vec![0u64; 50];
    let mut ring_idx = 0usize;

    for step in 0..10_000_000u64 {
        let pc_before = cpu.pc();

        clint.tick(1);
        if clint.timer_pending(0) {
            cpu.csr_mut().mip |= csr::MIP_MTIP;
        } else {
            cpu.csr_mut().mip &= !csr::MIP_MTIP;
        }

        // Check if we're about to enter sbi_hart_hang
        if pc_before == HANG_ADDR {
            eprintln!("=== sbi_hart_hang reached at step {} ===", step);
            eprintln!("  ra=0x{:X} (return address / caller)", cpu.reg(1));
            eprintln!(
                "  sp=0x{:X} a0=0x{:X} a1=0x{:X} a2=0x{:X}",
                cpu.reg(2),
                cpu.reg(10),
                cpu.reg(11),
                cpu.reg(12)
            );
            eprintln!(
                "  mcause=0x{:X} mepc=0x{:X}",
                cpu.csr().mcause,
                cpu.csr().mepc
            );
            eprintln!(
                "  mtvec=0x{:X} mstatus=0x{:X}",
                cpu.csr().mtvec,
                cpu.csr().mstatus
            );
            eprintln!("\n  Last 50 PCs:");
            for i in 0..50 {
                let idx = (ring_idx + i) % 50;
                if pc_ring[idx] != 0 {
                    let raw = bus_arc
                        .read(pc_ring[idx], MemAccessSize::U32, 0)
                        .unwrap_or(0);
                    eprintln!("    0x{:08X}  insn=0x{:08X}", pc_ring[idx], raw);
                }
            }
            break;
        }

        pc_ring[ring_idx] = pc_before;
        ring_idx = (ring_idx + 1) % 50;

        cpu.exec(1).unwrap();

        if cpu.is_halted() {
            eprintln!("HALTED at step {} PC=0x{:X}", step, cpu.pc());
            break;
        }
    }

    let output = uart.take_output();
    if !output.is_empty() {
        eprintln!("UART: {}", String::from_utf8_lossy(&output));
    }
}
