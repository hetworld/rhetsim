//! Trace Linux kernel early boot to diagnose PC stuck at entry point.

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemTarget};
use rhetsim_cpu_riscv::cpu::RiscvCpu;
use rhetsim_cpu_riscv::csr;
use std::sync::Arc;

const DRAM_BASE: u64 = 0x8000_0000;
const DRAM_SIZE: usize = 64 * 1024 * 1024;
const CLINT_BASE: u64 = 0x0200_0000;
const UART_BASE: u64 = 0x1000_0000;
const KERNEL_ADDR: u64 = 0x8020_0000;
const DTB_ADDR: u64 = 0x8200_0000;

#[test]
fn linux_trace() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let fw_path = workspace
        .parent()
        .unwrap()
        .join("opensbi/build/platform/generic/firmware/fw_jump.bin");
    let kernel_path = workspace
        .parent()
        .unwrap()
        .join("linux-6.1/arch/riscv/boot/Image");
    let dtb_path = workspace.join("platforms/reference/server/riscv-virt.dtb");

    let fw_data = match std::fs::read(&fw_path) {
        Ok(d) => d,
        Err(_) => {
            eprintln!("skip");
            return;
        }
    };
    let kernel_data = match std::fs::read(&kernel_path) {
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
    cpu.load_binary(&kernel_data, KERNEL_ADDR - DRAM_BASE);
    cpu.load_binary(&dtb_data, DTB_ADDR - DRAM_BASE);
    cpu.set_reg(10, 0);
    cpu.set_reg(11, DTB_ADDR);
    cpu.set_pc(DRAM_BASE);

    // Run OpenSBI to completion (until it jumps to kernel)
    let mut reached_kernel = false;
    let mut total_steps = 0u64;
    for step in 0..10_000_000u64 {
        clint.tick(1);
        if clint.timer_pending(0) {
            cpu.csr_mut().mip |= csr::MIP_MTIP;
        } else {
            cpu.csr_mut().mip &= !csr::MIP_MTIP;
        }

        cpu.exec(1).unwrap();
        total_steps += 1;

        if cpu.is_halted() {
            eprintln!("HALTED at step {} PC=0x{:X}", step, cpu.pc());
            break;
        }

        if !reached_kernel && cpu.pc() >= KERNEL_ADDR && cpu.pc() < KERNEL_ADDR + 0x200000 {
            reached_kernel = true;
            eprintln!("\n=== Entered kernel at step {} ===", step);
            eprintln!("  PC=0x{:X} mode={}", cpu.pc(), cpu.priv_mode());
            eprintln!(
                "  stvec=0x{:X} satp=0x{:X}",
                cpu.csr().stvec,
                cpu.csr().satp
            );
            eprintln!(
                "  mtvec=0x{:X} mstatus=0x{:X}",
                cpu.csr().mtvec,
                cpu.csr().mstatus
            );

            // Now trace kernel WITH CLINT ticking (like linux_boot test)
            let mut entry_count = 0u64;
            for k_step in 0..200_000u64 {
                let kpc = cpu.pc();
                let mode = cpu.priv_mode();

                // Tick CLINT same as linux_boot: 1 tick per instruction
                clint.tick(1);
                if clint.timer_pending(0) {
                    cpu.csr_mut().mip |= csr::MIP_MTIP;
                } else {
                    cpu.csr_mut().mip &= !csr::MIP_MTIP;
                }

                cpu.exec(1).unwrap();
                total_steps += 1;
                let next_pc = cpu.pc();

                // Detect when PC hits kernel entry
                if kpc == KERNEL_ADDR || next_pc == KERNEL_ADDR {
                    entry_count += 1;
                    if entry_count <= 5 {
                        eprintln!("  [k={}] KERNEL ENTRY #{}: 0x{:X} m={} -> 0x{:X}  scause=0x{:X} sepc=0x{:X} stval=0x{:X}",
                            k_step, entry_count, kpc, mode, next_pc,
                            cpu.csr().scause, cpu.csr().sepc, cpu.csr().stval);
                        eprintln!(
                            "         satp=0x{:X} stvec=0x{:X} mstatus=0x{:X}",
                            cpu.csr().satp,
                            cpu.csr().stvec,
                            cpu.csr().mstatus
                        );
                        // Dump the root page table
                        let satp_ppn = cpu.csr().satp & 0x0FFF_FFFF_FFFF;
                        let root_pa = satp_ppn * 4096;
                        eprintln!("         Root PT @ 0x{:X}:", root_pa);
                        for i in 0..4u64 {
                            let pte_addr = root_pa + i * 8;
                            let pte = bus_arc
                                .read(pte_addr, rhetsim_core::mem::MemAccessSize::U64, 0)
                                .unwrap_or(0);
                            if pte != 0 {
                                eprintln!("           [{}] pte=0x{:016X} V={} R={} W={} X={} U={} A={} D={} ppn=0x{:X}",
                                    i, pte, pte & 1, (pte>>1)&1, (pte>>2)&1, (pte>>3)&1, (pte>>4)&1, (pte>>6)&1, (pte>>7)&1, (pte>>10) & 0xFFF_FFFF_FFFF);
                            }
                        }
                        // Also check entries around VPN[2]=2 (for 0x8xxxxxxx addresses)
                        for i in [2u64, 256, 510, 511] {
                            let pte_addr = root_pa + i * 8;
                            let pte = bus_arc
                                .read(pte_addr, rhetsim_core::mem::MemAccessSize::U64, 0)
                                .unwrap_or(0);
                            if pte != 0 {
                                eprintln!("           [{}] pte=0x{:016X} V={} R={} W={} X={} U={} A={} D={} ppn=0x{:X}",
                                    i, pte, pte & 1, (pte>>1)&1, (pte>>2)&1, (pte>>3)&1, (pte>>4)&1, (pte>>6)&1, (pte>>7)&1, (pte>>10) & 0xFFF_FFFF_FFFF);
                            }
                        }
                    }
                    if entry_count >= 5 {
                        eprintln!("  Kernel re-entered 5 times. Timer interrupt loop confirmed.");
                        break;
                    }
                }

                // Print first 20 instructions and any mode transitions
                if k_step < 20 {
                    eprintln!(
                        "  [k={}] 0x{:08X} m={} -> 0x{:08X}",
                        k_step, kpc, mode, next_pc
                    );
                }

                // Print when switching between S-mode and M-mode
                if mode != cpu.priv_mode() && k_step >= 20 {
                    eprintln!("  [k={}] MODE CHANGE: 0x{:08X} m={} -> 0x{:08X} m={}  mcause=0x{:X} mepc=0x{:X}",
                        k_step, kpc, mode, next_pc, cpu.priv_mode(),
                        cpu.csr().mcause, cpu.csr().mepc);
                }

                if cpu.is_halted() {
                    eprintln!("  HALTED at k_step={}", k_step);
                    break;
                }
            }
            break;
        }
    }

    if !reached_kernel {
        eprintln!(
            "Never reached kernel after {} steps! PC=0x{:X} mode={}",
            total_steps,
            cpu.pc(),
            cpu.priv_mode()
        );
    }
}
