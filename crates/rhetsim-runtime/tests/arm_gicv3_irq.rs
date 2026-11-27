//! Integration test: ARM CPU + GICv3 interrupt-driven program
//!
//! Demonstrates the full IRQ flow:
//!   1. ARM CPU sets VBAR_EL1 and enables GIC via ICC system registers
//!   2. CPU enters poll loop waiting for a flag
//!   3. Test harness raises SPI 32 on the GIC
//!   4. CPU takes IRQ exception, vectors to VBAR+0x480
//!   5. Handler acknowledges (ICC_IAR1), sets flag, EOIs (ICC_EOIR1), ERETurns
//!   6. CPU resumes, sees flag set, halts via SVC

use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::irq::IrqSink;
use rhetsim_core::mem::{MemAccessSize, MemAttrs, MemTarget};
use rhetsim_cpu_arm::cpu::ArmCpu;
use rhetsim_intc_gicv3::Gicv3;
use rhetsim_mem_dram::Dram;
use std::sync::Arc;

const DRAM_BASE: u64 = 0x4000_0000;
const DRAM_SIZE: usize = 0x1_0000; // 64 KiB
const GIC_BASE: u64 = 0x0800_0000;

#[test]
fn arm_gicv3_spi_interrupt() {
    let dram = Arc::new(Dram::with_base("dram0", DRAM_SIZE, DRAM_BASE));
    let gic = Arc::new(Gicv3::new("gic0", 1, 64, GIC_BASE));

    let mut bus = MemBus::new("sys-bus");
    bus.map(
        dram.clone() as Arc<dyn MemTarget>,
        DRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        gic.clone() as Arc<dyn MemTarget>,
        GIC_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus = Arc::new(bus);

    let mut cpu = ArmCpu::with_bus("arm0", bus.clone(), DRAM_BASE);
    cpu.attach_gic(gic.clone(), 0);
    cpu.set_sp(DRAM_BASE + DRAM_SIZE as u64);

    // ── Main code at DRAM_BASE ────────────────────────────────────
    //   MOVZ X0, #0x2000
    //   MOVK X0, #0x4000, lsl #16   -> X0 = VBAR = 0x4000_2000
    //   MSR  VBAR_EL1, X0
    //   MOVZ X0, #1
    //   MSR  ICC_IGRPEN1_EL1, X0    -> enable Group 1
    //   MOVZ X0, #0xFF
    //   MSR  ICC_PMR_EL1, X0        -> PMR = 0xFF (allow all priorities)
    //   MOVZ X1, #0x100
    //   MOVK X1, #0x4000, lsl #16   -> X1 = flag addr = 0x4000_0100
    //   STR  XZR, [X1]              -> flag = 0
    // poll:
    //   LDR  X2, [X1]
    //   CBZ  X2, poll
    //   SVC  #0
    let main_code: Vec<u32> = vec![
        0xD2840000, // MOVZ X0, #0x2000
        0xF2A80000, // MOVK X0, #0x4000, lsl #16
        0xD518C000, // MSR VBAR_EL1, X0
        0xD2800020, // MOVZ X0, #1
        0xD518CCE0, // MSR ICC_IGRPEN1_EL1, X0
        0xD2801FE0, // MOVZ X0, #0xFF
        0xD5184600, // MSR ICC_PMR_EL1, X0
        0xD2802001, // MOVZ X1, #0x100
        0xF2A80001, // MOVK X1, #0x4000, lsl #16
        0xF900003F, // STR XZR, [X1]
        0xF9400022, // LDR X2, [X1]        (poll)
        0xB4FFFFE2, // CBZ X2, -4          (back to LDR)
        0xD4000001, // SVC #0
    ];
    let main_bytes: Vec<u8> = main_code.iter().flat_map(|w| w.to_le_bytes()).collect();
    for (i, &b) in main_bytes.iter().enumerate() {
        bus.write(DRAM_BASE + i as u64, MemAccessSize::U8, b as u64, 0)
            .unwrap();
    }

    // ── IRQ handler at VBAR + 0x480 = DRAM_BASE + 0x2480 ─────────
    //   MRS  X10, ICC_IAR1_EL1       -> acknowledge, X10 = INTID
    //   MOVZ X11, #0x100
    //   MOVK X11, #0x4000, lsl #16   -> X11 = flag addr
    //   MOVZ X12, #1
    //   STR  X12, [X11]              -> flag = 1
    //   MSR  ICC_EOIR1_EL1, X10      -> end-of-interrupt
    //   ERET
    let handler_code: Vec<u32> = vec![
        0xD538CC0A, // MRS X10, ICC_IAR1_EL1
        0xD280200B, // MOVZ X11, #0x100
        0xF2A8000B, // MOVK X11, #0x4000, lsl #16
        0xD280002C, // MOVZ X12, #1
        0xF900016C, // STR X12, [X11]
        0xD518CC2A, // MSR ICC_EOIR1_EL1, X10
        0xD69F03E0, // ERET
    ];
    let handler_bytes: Vec<u8> = handler_code.iter().flat_map(|w| w.to_le_bytes()).collect();
    let handler_offset = 0x2480u64;
    for (i, &b) in handler_bytes.iter().enumerate() {
        bus.write(
            DRAM_BASE + handler_offset + i as u64,
            MemAccessSize::U8,
            b as u64,
            0,
        )
        .unwrap();
    }

    // ── Configure GIC: enable SPI 32, priority 0x40, routed to CPU 0 ──
    gic.configure_spi(32, 0x40, 0);

    // Phase 1: run setup + enter poll loop
    cpu.exec(15).unwrap();
    assert!(!cpu.is_halted(), "CPU should be polling, not halted");

    // Phase 2: raise SPI 32 externally
    gic.raise(32).unwrap();
    assert!(gic.pending_irq(0).is_some(), "SPI 32 should be pending");

    // Phase 3: handler fires, sets flag, ERET, main sees flag and halts
    cpu.exec(30).unwrap();

    assert!(cpu.is_halted(), "CPU should have halted via SVC");
    assert_eq!(cpu.reg(10), 32, "X10 should hold acknowledged INTID = 32");
    assert_eq!(cpu.reg(12), 1, "X12 = flag value = 1");
}
