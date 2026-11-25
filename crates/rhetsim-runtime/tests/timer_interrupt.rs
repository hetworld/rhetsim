//! End-to-end test: CLINT timer interrupt fires, trap handler executes, MRET returns.
//!
//! Platform layout (base 0 to avoid RV64 LUI sign-extension):
//!   DRAM at 0x0000_0000 (64 KiB) — code + data
//!   CLINT at 0x0200_0000
//!
//! Test flow:
//!   1. Main code sets mtvec, enables MTIE, writes mtimecmp=50 on CLINT
//!   2. CPU runs; each step ticks CLINT by 1
//!   3. At mtime >= 50, timer interrupt fires → handler sets x10=1, disables MTIE, MRETs
//!   4. Main code continues, sets x11=42, ECALLs

use std::sync::Arc;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemTarget};
use rhetsim_cpu_riscv::cpu::RiscvCpu;
use rhetsim_cpu_riscv::csr;

const MEM_BASE: u64 = 0;
const CLINT_BASE: u64 = 0x0200_0000;
const HANDLER_OFFSET: u64 = 0x400;

fn csrrw(csr: u16, rs1: u8, rd: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (1 << 12) | ((rd as u32) << 7) | 0x73
}
fn csrrs(csr: u16, rs1: u8, rd: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (2 << 12) | ((rd as u32) << 7) | 0x73
}
fn csrrc(csr: u16, rs1: u8, rd: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (3 << 12) | ((rd as u32) << 7) | 0x73
}

fn build_main_code() -> Vec<u32> {
    vec![
        // x1 = HANDLER_OFFSET (0x400)
        0x40000093, // ADDI x1, x0, 0x400

        // CSRRW x0, mtvec, x1
        csrrw(0x305, 1, 0),

        // Build CLINT mtimecmp address: CLINT_BASE + 0x4000 = 0x0200_4000
        // LUI x2, 0x0200 → x2 = 0x0020_0000... no, LUI shifts by 12
        // LUI x2, 0x2004 → x2 = 0x0200_4000
        ((0x02004u32) << 12) | (2 << 7) | 0x37, // LUI x2, 0x2004

        // x3 = 50 (mtimecmp value)
        0x03200193u32, // ADDI x3, x0, 50

        // SD x3, 0(x2) — write mtimecmp
        (0 << 25) | (3 << 20) | (2 << 15) | (3 << 12) | (0 << 7) | 0x23, // SD x3, 0(x2)

        // Enable MTIE (bit 7 = 128) in mie
        0x08000313, // ADDI x6, x0, 128
        csrrs(0x304, 6, 0),

        // Enable global MIE (bit 3 = 8) in mstatus
        0x00800393, // ADDI x7, x0, 8
        csrrs(0x300, 7, 0),

        // NOP sled — interrupt will fire during one of these
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,
        0x00000013, 0x00000013, 0x00000013, 0x00000013, 0x00000013,

        // After returning from interrupt handler:
        0x02A00593, // ADDI x11, x0, 42
        csrrw(0x305, 0, 0), // clear mtvec so ECALL halts
        0x00000073, // ECALL
    ]
}

fn build_handler_code() -> Vec<u32> {
    vec![
        0x00100513, // ADDI x10, x0, 1    — marker: handler ran
        0x08000493, // ADDI x9, x0, 128   — MTIE bit
        csrrc(0x304, 9, 0), // CSRRC x0, mie, x9  — disable MTIE
        0x30200073, // MRET
    ]
}

#[test]
fn timer_interrupt_end_to_end() {
    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram0", 64 * 1024, MEM_BASE));
    let clint = Arc::new(rhetsim_intc_clint::Clint::new("clint0", 1, CLINT_BASE));

    let mut bus = MemBus::new("sys-bus");
    bus.map(dram.clone() as Arc<dyn MemTarget>, MEM_BASE, MemAttrs::rw(), None).unwrap();
    bus.map(clint.clone() as Arc<dyn MemTarget>, CLINT_BASE, MemAttrs::rw(), None).unwrap();
    let bus = Arc::new(bus);

    let mut cpu = RiscvCpu::with_bus("rv-core0", bus, MEM_BASE);

    // Load main code at MEM_BASE
    let main_code = build_main_code();
    for (i, insn) in main_code.iter().enumerate() {
        cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
    }

    // Load handler at HANDLER_OFFSET
    let handler_code = build_handler_code();
    for (i, insn) in handler_code.iter().enumerate() {
        cpu.load_binary(&insn.to_le_bytes(), HANDLER_OFFSET + (i * 4) as u64);
    }

    // Tick CLINT past mtimecmp before entering the exec loop so the
    // interrupt is already pending when check_interrupt runs.  This
    // decouples the test from JIT basic-block granularity (a JIT block
    // may execute many instructions before returning to the exec loop
    // where interrupt checks happen).
    clint.tick(60);
    cpu.csr_mut().mip |= csr::MIP_MTIP;

    let mut steps = 0u64;
    while !cpu.is_halted() && steps < 500 {
        cpu.exec(1).unwrap();
        steps += 1;
        if clint.timer_pending(0) {
            cpu.csr_mut().mip |= csr::MIP_MTIP;
        } else {
            cpu.csr_mut().mip &= !csr::MIP_MTIP;
        }
    }

    assert!(cpu.is_halted(), "CPU should halt via ECALL (ran {} steps)", steps);
    assert_eq!(cpu.reg(10), 1, "trap handler should have set x10=1");
    assert_eq!(cpu.reg(11), 42, "main code should have set x11=42 after MRET");
}
