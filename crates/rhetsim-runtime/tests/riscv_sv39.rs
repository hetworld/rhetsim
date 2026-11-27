//! Integration test: RISC-V S-mode with Sv39 MMU
//!
//! Flow:
//!   1. M-mode: build Sv39 page table, set medeleg to delegate ecalls, set satp
//!   2. M-mode: set mepc to S-mode entry (virtual addr), MPP=S, MRET
//!   3. S-mode: store/load through page table, ECALL → traps to S-mode (delegated)
//!   4. S-mode trap handler: sets marker, advances sepc, SRET
//!   5. S-mode: clear stvec, ECALL → delegated but stvec=0 → halt

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_cpu_riscv::cpu::RiscvCpu;

const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;

fn pte(ppn: u64, flags: u64) -> u64 {
    (ppn << 10) | flags
}

fn csrrw(csr: u16, rs1: u8, rd: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (1 << 12) | ((rd as u32) << 7) | 0x73
}
fn csrrs(csr: u16, rs1: u8, rd: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (2 << 12) | ((rd as u32) << 7) | 0x73
}

fn emit(cpu: &mut RiscvCpu, offset: u64, insns: &[u32]) {
    for (i, insn) in insns.iter().enumerate() {
        cpu.load_binary(&insn.to_le_bytes(), offset + (i * 4) as u64);
    }
}

fn write_u64(cpu: &mut RiscvCpu, offset: u64, val: u64) {
    cpu.load_binary(&val.to_le_bytes(), offset);
}

#[test]
fn sv39_m_to_s_with_paging() {
    let mut cpu = RiscvCpu::new("rv0", 0x1_0000, 0);
    // PMP default-denies S/U accesses without a matching entry; cover the
    // whole address space like firmware would.
    cpu.csr_mut().pmpaddr[0] = (1 << 54) - 1;
    cpu.csr_mut().pmpcfg[0] = 0x1F; // NAPOT | R | W | X

    // ── Page tables ───────────────────────────────────────
    // PA layout: M-code@0x0, L2@0x1000, L1@0x2000, L0@0x3000,
    //            S-code@0x4000, S-data@0x5000, S-handler@0x6000
    // VA map: 0x10_0000→0x4000, 0x10_1000→0x5000, 0x10_2000→0x6000
    let l1_ppn: u64 = 2;
    let l0_ppn: u64 = 3;

    // L2[0] → L1
    write_u64(&mut cpu, 0x1000, pte(l1_ppn, PTE_V));
    // L1[0] → L0
    write_u64(&mut cpu, 0x2000, pte(l0_ppn, PTE_V));
    // L0[0x100] → code (RX)
    write_u64(
        &mut cpu,
        0x3000 + 0x100 * 8,
        pte(0x4, PTE_V | PTE_R | PTE_X | PTE_A | PTE_D),
    );
    // L0[0x101] → data (RW)
    write_u64(
        &mut cpu,
        0x3000 + 0x101 * 8,
        pte(0x5, PTE_V | PTE_R | PTE_W | PTE_A | PTE_D),
    );
    // L0[0x102] → handler (RX)
    write_u64(
        &mut cpu,
        0x3000 + 0x102 * 8,
        pte(0x6, PTE_V | PTE_R | PTE_X | PTE_A | PTE_D),
    );

    // ── S-mode code at PA 0x4000 (VA 0x10_0000) ──────────
    let s_code: Vec<u32> = vec![
        0x00101537, // LUI x10, 0x101 → x10 = 0x10_1000 (data VA)
        0x02A00593, // ADDI x11, x0, 42
        // SW x11, 0(x10)
        (11 << 20) | (10 << 15) | (2 << 12) | 0x23,
        // LW x12, 0(x10)
        (10 << 15) | (2 << 12) | (12 << 7) | 0x03,
        // ECALL → delegated to S-mode handler
        0x00000073,
        // After SRET: clear stvec, then ECALL → halt
        csrrw(0x105, 0, 0),
        0x00000073,
    ];
    emit(&mut cpu, 0x4000, &s_code);

    // ── S-mode trap handler at PA 0x6000 (VA 0x10_2000) ──
    let s_handler: Vec<u32> = vec![
        0x00100693,          // ADDI x13, x0, 1
        csrrs(0x141, 0, 14), // read sepc → x14
        0x00470713,          // ADDI x14, x14, 4
        csrrw(0x141, 14, 0), // write sepc ← x14
        0x10200073,          // SRET
    ];
    emit(&mut cpu, 0x6000, &s_handler);

    // ── M-mode boot at PA 0x0000 ─────────────────────────
    let lui_x5_1: u32 = (1 << 12) | (5 << 7) | 0x37;
    let srli_x5_1: u32 = (1 << 20) | (5 << 15) | (5 << 12) | (5 << 7) | 0x13;

    let m_code: Vec<u32> = vec![
        // stvec = 0x10_2000
        0x00102137, // LUI x2, 0x102
        csrrw(0x105, 2, 0),
        // medeleg |= (1 << 9) — delegate ecall-from-S
        0x20000193, // ADDI x3, x0, 0x200
        csrrs(0x302, 3, 0),
        // mepc = 0x10_0000
        0x00100237, // LUI x4, 0x100
        csrrw(0x341, 4, 0),
        // mstatus.MPP = S (0x800)
        lui_x5_1,  // LUI x5, 1 → 0x1000
        srli_x5_1, // SRLI x5, x5, 1 → 0x800
        csrrs(0x300, 5, 0),
        // satp = (8 << 60) | 1
        0x00800313,                                           // ADDI x6, x0, 8
        (60 << 20) | (6 << 15) | (1 << 12) | (6 << 7) | 0x13, // SLLI x6, x6, 60
        0x00136313,                                           // ORI x6, x6, 1
        csrrw(0x180, 6, 0),
        // MRET → S-mode
        0x30200073,
    ];
    emit(&mut cpu, 0x0000, &m_code);

    // ── Run ───────────────────────────────────────────────
    cpu.set_pc(0);
    cpu.exec(200).unwrap();

    assert!(cpu.is_halted(), "CPU should halt");
    assert_eq!(cpu.reg(11), 42, "stored 42 through Sv39 page table");
    assert_eq!(cpu.reg(12), 42, "loaded 42 back through Sv39 page table");
    assert_eq!(cpu.reg(13), 1, "S-mode trap handler ran (ecall delegation)");
    assert_eq!(cpu.csr().scause, 9, "scause = ecall from S-mode");
}
