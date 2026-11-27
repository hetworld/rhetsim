//! Three-way heterogeneous platform: ARM + RISC-V + GPGPU
//!
//! Memory map:
//!   0x0000_0000 — shared DRAM (64 KiB, data + RV code at 0xA000)
//!   0x4000_0000 — GPGPU control registers (256 bytes)
//!   0x4000_1000 — GPGPU VRAM (64 KiB)
//!   ARM code loaded at DRAM 0x8000
//!   RISC-V code loaded at DRAM 0xA000
//!
//! Flow:
//!   1. ARM host writes vec_a [1,2,3,4] and vec_b [10,20,30,40] to GPGPU VRAM,
//!      configures vector-add command, rings doorbell
//!   2. GPGPU executes vector addition → result [11,22,33,44] in VRAM+0x2000
//!   3. RISC-V coprocessor reads result from VRAM, sums all elements,
//!      writes total (110) to shared DRAM offset 0x100

use rhetsim_accel_gpgpu::Gpgpu;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAccessSize, MemAttrs, MemTarget};
use rhetsim_cpu_arm::cpu::ArmCpu;
use rhetsim_cpu_riscv::cpu::RiscvCpu;
use std::sync::Arc;

const DRAM_BASE: u64 = 0;
const DRAM_SIZE: usize = 64 * 1024;
const GPU_CTRL_BASE: u64 = 0x4000_0000;
const GPU_VRAM_BASE: u64 = 0x4000_1000;
const GPU_VRAM_SIZE: usize = 64 * 1024;

fn build_arm_gpu_writer() -> Vec<u32> {
    vec![
        // X1 = 0x4000_1000 (VRAM base)
        0xD2820001, // MOVZ X1, #0x1000
        0xF2A80001, // MOVK X1, #0x4000, lsl #16
        // X2 = 0x4000_0000 (GPU CTRL base)
        0xD2800002, // MOVZ X2, #0
        0xF2A80002, // MOVK X2, #0x4000, lsl #16
        // X3 = 0x4000_2000 (VRAM+0x1000, for vec_b)
        0xD2840003, // MOVZ X3, #0x2000
        0xF2A80003, // MOVK X3, #0x4000, lsl #16
        // Write vec_a [1,2,3,4] to VRAM+0x0000
        0xD2800020, // MOVZ X0, #1
        0xB9000020, // STR W0, [X1, #0]
        0xD2800040, // MOVZ X0, #2
        0xB9000420, // STR W0, [X1, #4]
        0xD2800060, // MOVZ X0, #3
        0xB9000820, // STR W0, [X1, #8]
        0xD2800080, // MOVZ X0, #4
        0xB9000C20, // STR W0, [X1, #12]
        // Write vec_b [10,20,30,40] to VRAM+0x1000
        0xD2800140, // MOVZ X0, #10
        0xB9000060, // STR W0, [X3, #0]
        0xD2800280, // MOVZ X0, #20
        0xB9000460, // STR W0, [X3, #4]
        0xD28003C0, // MOVZ X0, #30
        0xB9000860, // STR W0, [X3, #8]
        0xD2800500, // MOVZ X0, #40
        0xB9000C60, // STR W0, [X3, #12]
        // Configure GPU: OP=1(VECTOR_ADD), LEN=4, SRC_A=0, SRC_B=0x1000, DST=0x2000
        0xD2800020, // MOVZ X0, #1
        0xB9000840, // STR W0, [X2, #8]   CMD_OP
        0xD2800080, // MOVZ X0, #4
        0xB9000C40, // STR W0, [X2, #12]  CMD_LEN
        0xB900105F, // STR WZR, [X2, #16] CMD_SRC_A = 0
        0xD2820000, // MOVZ X0, #0x1000
        0xB9001440, // STR W0, [X2, #20]  CMD_SRC_B
        0xD2840000, // MOVZ X0, #0x2000
        0xB9001840, // STR W0, [X2, #24]  CMD_DST
        0xD2800020, // MOVZ X0, #1
        0xB9000040, // STR W0, [X2, #0]   DOORBELL
        // Halt
        0xD4000001, // SVC #0
    ]
}

fn build_rv_result_reader() -> Vec<u32> {
    // Reads 4 u32 results from GPU VRAM+0x2000 (bus 0x4000_3000),
    // sums them, stores total to DRAM 0x100.
    //   x2 = 0x4000_3000 (VRAM result base)
    //   x1 = accumulator
    //   x3 = temp
    //   x6 = 0x100 (result address)
    vec![
        0x40003137, // LUI x2, 0x40003    -> x2 = 0x4000_3000
        0x00012183, // LW  x3, 0(x2)      -> result[0]
        0x003000B3, // ADD x1, x0, x3     -> acc = result[0]
        0x00412183, // LW  x3, 4(x2)      -> result[1]
        0x003080B3, // ADD x1, x1, x3     -> acc += result[1]
        0x00812183, // LW  x3, 8(x2)      -> result[2]
        0x003080B3, // ADD x1, x1, x3     -> acc += result[2]
        0x00C12183, // LW  x3, 12(x2)     -> result[3]
        0x003080B3, // ADD x1, x1, x3     -> acc += result[3]
        0x10000313, // ADDI x6, x0, 0x100 -> x6 = result addr
        0x00133023, // SD   x1, 0(x6)     -> store sum
        0x00000073, // ECALL              -> halt
    ]
}

#[test]
fn three_way_arm_rv_gpgpu() {
    let gpu = Gpgpu::new("gpu0", GPU_VRAM_SIZE);
    let gpu_ctrl = gpu.ctrl_arc();
    let gpu_vram = gpu.vram_arc();

    let dram = Arc::new(rhetsim_mem_dram::Dram::with_base(
        "dram0", DRAM_SIZE, DRAM_BASE,
    ));

    let mut bus = MemBus::new("sys-bus");
    bus.map(
        dram.clone() as Arc<dyn MemTarget>,
        DRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        gpu_ctrl as Arc<dyn MemTarget>,
        GPU_CTRL_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    bus.map(
        gpu_vram.clone() as Arc<dyn MemTarget>,
        GPU_VRAM_BASE,
        MemAttrs::rw(),
        None,
    )
    .unwrap();
    let bus = Arc::new(bus);

    // ── Load ARM code at DRAM offset 0x8000 ───────────────────────
    let arm_code = build_arm_gpu_writer();
    let arm_code_base = 0x8000u64;
    for (i, insn) in arm_code.iter().enumerate() {
        bus.write(
            arm_code_base + (i * 4) as u64,
            MemAccessSize::U32,
            *insn as u64,
            0,
        )
        .unwrap();
    }

    // ── Load RISC-V code at DRAM offset 0xA000 ───────────────────
    let rv_code = build_rv_result_reader();
    let rv_code_base = 0xA000u64;
    for (i, insn) in rv_code.iter().enumerate() {
        bus.write(
            rv_code_base + (i * 4) as u64,
            MemAccessSize::U32,
            *insn as u64,
            0,
        )
        .unwrap();
    }

    // ── Phase 1: ARM writes vectors to VRAM, configures GPU ──────
    let mut arm = ArmCpu::with_bus("arm-host", bus.clone(), arm_code_base);
    arm.set_sp(0x7F00);
    arm.exec(100).unwrap();
    assert!(arm.is_halted(), "ARM should halt via SVC");

    // Verify doorbell was rung
    assert!(gpu.ctrl().has_pending(), "GPU should have pending command");

    // Verify vec_a was written to VRAM
    assert_eq!(gpu_vram.read_u32(0x0000).unwrap(), 1);
    assert_eq!(gpu_vram.read_u32(0x0004).unwrap(), 2);
    assert_eq!(gpu_vram.read_u32(0x0008).unwrap(), 3);
    assert_eq!(gpu_vram.read_u32(0x000C).unwrap(), 4);

    // ── Phase 2: GPGPU processes vector addition ─────────────────
    let mut gpu = gpu;
    gpu.exec(100).unwrap();

    // Verify result in VRAM+0x2000: [11, 22, 33, 44]
    assert_eq!(gpu_vram.read_u32(0x2000).unwrap(), 11);
    assert_eq!(gpu_vram.read_u32(0x2004).unwrap(), 22);
    assert_eq!(gpu_vram.read_u32(0x2008).unwrap(), 33);
    assert_eq!(gpu_vram.read_u32(0x200C).unwrap(), 44);

    // ── Phase 3: RISC-V reads result, sums, stores to DRAM ──────
    let mut rv = RiscvCpu::with_bus("rv-coproc", bus.clone(), rv_code_base);
    rv.exec(100).unwrap();
    assert!(rv.is_halted(), "RISC-V should halt via ECALL");

    // ── Verify: sum = 11+22+33+44 = 110 at DRAM 0x100 ───────────
    let sum = bus.read(0x100, MemAccessSize::U64, 0).unwrap();
    assert_eq!(
        sum, 110,
        "Three-way heterogeneous result: 11+22+33+44 = 110"
    );
}
