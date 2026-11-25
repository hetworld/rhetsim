//! End-to-end: AArch64 host + RISC-V coprocessor on shared MemBus.
//!
//! Memory map:
//!   0x0000_0000 — shared DRAM (64 KiB, data exchange area)
//!   0x1000_0000 — ARM private code memory (via flat backend, not on bus)
//!   0x2000_0000 — RISC-V private code memory (via flat backend, not on bus)
//!
//! Test flow:
//!   1. ARM host writes four u64 values [10, 20, 30, 40] to shared DRAM at offset 0
//!   2. RISC-V coprocessor reads those values, sums them, writes result to DRAM offset 0x100
//!   3. ARM host reads the result from DRAM offset 0x100 and verifies it equals 100

use std::sync::Arc;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAttrs, MemTarget, MemAccessSize};

const SHARED_DRAM_BASE: u64 = 0;
const SHARED_DRAM_SIZE: usize = 64 * 1024;

fn build_arm_writer() -> Vec<u32> {
    // AArch64 program that writes [10, 20, 30, 40] as u64s to DRAM at offsets 0, 8, 16, 24
    // then stores 1 to offset 0x80 as a "ready" flag, then halts via SVC.
    // X0 stays 0 (base address), X1 is the value register.
    vec![
        0xD2800141, // MOVZ X1, #10
        0xF9000001, // STR X1, [X0, #0]
        0xD2800281, // MOVZ X1, #20
        0xF9000401, // STR X1, [X0, #8]
        0xD28003C1, // MOVZ X1, #30
        0xF9000801, // STR X1, [X0, #16]
        0xD2800501, // MOVZ X1, #40
        0xF9000C01, // STR X1, [X0, #24]
        0xD2800021, // MOVZ X1, #1
        0xD2801002, // MOVZ X2, #0x80
        0xF9000041, // STR X1, [X2, #0]
        0xD4000001, // SVC #0
    ]
}

fn build_riscv_summer() -> Vec<u32> {
    // RV64I program that reads 4 u64s from DRAM at offsets 0..24, sums them,
    // and stores the result at offset 0x100, then ECALLs.
    vec![
        // LD x1, 0(x0)   — load DRAM[0]
        0x00003083u32, // LD x1, 0(x0)
        // LD x2, 8(x0)
        0x00803103u32, // LD x2, 8(x0)
        // LD x3, 16(x0)
        0x01003183u32, // LD x3, 16(x0)
        // LD x4, 24(x0)
        0x01803203u32, // LD x4, 24(x0)
        // x5 = x1 + x2
        0x002082B3u32, // ADD x5, x1, x2
        // x5 = x5 + x3
        0x003282B3u32, // ADD x5, x5, x3
        // x5 = x5 + x4
        0x004282B3u32, // ADD x5, x5, x4
        // Store result at offset 0x100
        // x6 = 0x100
        0x10000313u32, // ADDI x6, x0, 256
        // SD x5, 0(x6)
        0x00533023u32, // SD x5, 0(x6)
        // ECALL
        0x00000073u32,
    ]
}

#[test]
fn hetero_arm_riscv_shared_dram() {
    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base(
        "shared-dram", SHARED_DRAM_SIZE, SHARED_DRAM_BASE,
    ));

    let mut bus = MemBus::new("sys-bus");
    bus.map(dram.clone() as Arc<dyn MemTarget>, SHARED_DRAM_BASE, MemAttrs::rw(), None).unwrap();
    let bus = Arc::new(bus);

    // ── Phase 1: ARM host writes data to shared DRAM ──────────
    let mut arm = rhetsim_cpu_arm::cpu::ArmCpu::with_bus("arm-host", bus.clone(), 0);
    // ARM code is loaded into a separate flat memory region, but it accesses shared DRAM via bus.
    // We use a flat-memory ARM CPU, load the code into its local memory, and have
    // stores go to bus addresses (which the bus routes to shared DRAM).
    // Actually, ArmCpu::with_bus uses the bus for ALL memory access, so code must be on the bus too.
    // Load ARM code into shared DRAM at a high offset (0x8000) to avoid data area.
    let arm_code = build_arm_writer();
    let arm_code_base = 0x8000u64;
    for (i, insn) in arm_code.iter().enumerate() {
        bus.write(arm_code_base + (i * 4) as u64, MemAccessSize::U32, *insn as u64, 0).unwrap();
    }
    arm.set_pc(arm_code_base);
    arm.set_sp(0xF000);

    let mut steps = 0;
    while !arm.is_halted() && steps < 100 {
        arm.exec(1).unwrap();
        steps += 1;
    }
    assert!(arm.is_halted(), "ARM should halt via SVC");
    assert!(steps <= 20, "ARM program should complete quickly (got {} steps)", steps);

    // Verify data was written to shared DRAM
    assert_eq!(bus.read(0, MemAccessSize::U64, 0).unwrap(), 10);
    assert_eq!(bus.read(8, MemAccessSize::U64, 0).unwrap(), 20);
    assert_eq!(bus.read(16, MemAccessSize::U64, 0).unwrap(), 30);
    assert_eq!(bus.read(24, MemAccessSize::U64, 0).unwrap(), 40);
    assert_eq!(bus.read(0x80, MemAccessSize::U64, 0).unwrap(), 1, "ready flag");

    // ── Phase 2: RISC-V coprocessor reads data, computes sum ──
    let rv_code = build_riscv_summer();
    let rv_code_base = 0xA000u64;
    for (i, insn) in rv_code.iter().enumerate() {
        bus.write(rv_code_base + (i * 4) as u64, MemAccessSize::U32, *insn as u64, 0).unwrap();
    }

    let mut rv = rhetsim_cpu_riscv::cpu::RiscvCpu::with_bus("rv-coproc", bus.clone(), rv_code_base);

    let mut rv_steps = 0;
    while !rv.is_halted() && rv_steps < 100 {
        rv.exec(1).unwrap();
        rv_steps += 1;
    }
    assert!(rv.is_halted(), "RISC-V should halt via ECALL");

    // x5 should contain 10+20+30+40 = 100
    assert_eq!(rv.reg(5), 100, "RISC-V sum should be 100");

    // Result should be written to DRAM at offset 0x100
    let result = bus.read(0x100, MemAccessSize::U64, 0).unwrap();
    assert_eq!(result, 100, "shared DRAM at 0x100 should contain the sum 100");

    // ── Phase 3: ARM host reads back the result ───────────────
    // Build a small ARM program that reads from 0x100 into X10
    let arm_reader: Vec<u32> = vec![
        0xD2802003, // MOVZ X3, #0x100
        0xF940006A, // LDR X10, [X3, #0]
        0xD4000001, // SVC #0
    ];
    let arm_read_base = 0xC000u64;
    for (i, insn) in arm_reader.iter().enumerate() {
        bus.write(arm_read_base + (i * 4) as u64, MemAccessSize::U32, *insn as u64, 0).unwrap();
    }

    let mut arm2 = rhetsim_cpu_arm::cpu::ArmCpu::with_bus("arm-reader", bus.clone(), arm_read_base);
    arm2.set_pc(arm_read_base);

    let mut steps2 = 0;
    while !arm2.is_halted() && steps2 < 100 {
        arm2.exec(1).unwrap();
        steps2 += 1;
    }
    assert!(arm2.is_halted(), "ARM reader should halt via SVC");
    assert_eq!(arm2.reg(10), 100, "ARM should read back sum=100 from shared DRAM");
}
