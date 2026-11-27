//! End-to-end: RISC-V CPU dispatches a compute kernel to GPGPU via SIMT
//!
//! Memory map:
//!   0x0000_0000 — shared DRAM (64 KiB, RV code at 0x0000)
//!   0x4000_0000 — GPGPU control registers (256 bytes)
//!   0x4000_1000 — GPGPU VRAM (64 KiB)
//!
//! Flow:
//!   1. RISC-V host CPU writes input vectors to GPGPU VRAM
//!   2. RISC-V host loads RV64I kernel code into GPGPU VRAM
//!   3. RISC-V host configures GPU: OP=KERNEL, kern_addr, grid/block dims, doorbell
//!   4. GPGPU SIMT-executes the kernel across multiple warps/SMs
//!   5. RISC-V host reads results from GPGPU VRAM, verifies sum

use rhetsim_accel_gpgpu::Gpgpu;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::{MemAccessSize, MemAttrs, MemTarget};
use rhetsim_cpu_riscv::cpu::RiscvCpu;
use std::sync::Arc;

const DRAM_BASE: u64 = 0;
const DRAM_SIZE: usize = 64 * 1024;
const GPU_CTRL_BASE: u64 = 0x4000_0000;
const GPU_VRAM_BASE: u64 = 0x4000_1000;
const GPU_VRAM_SIZE: usize = 64 * 1024;

fn build_host_program() -> Vec<u32> {
    // The RV host CPU:
    //   1. Writes vec_a [1,2,3,4,5,6,7,8] to GPU VRAM+0x4000
    //   2. Writes vec_b [10,20,30,40,50,60,70,80] to GPU VRAM+0x5000
    //   3. Writes kernel code to GPU VRAM+0x0000
    //   4. Configures GPU command registers and rings doorbell
    //   5. Reads results from GPU VRAM+0x6000, sums them, stores to DRAM 0x200
    //
    // GPU VRAM is at bus 0x4000_1000, so VRAM+0x4000 = bus 0x4000_5000
    // GPU CTRL is at bus 0x4000_0000

    // For simplicity, we'll write data using the CPU's bus-backed memory access.
    // The host program is a sequence of ADDI/LUI/SW to set up everything.

    let mut code: Vec<u32> = Vec::new();

    // ── Write vec_a to VRAM+0x4000 (bus 0x4000_5000) ──
    // x20 = 0x4000_5000
    code.push(0x40005A37); // LUI x20, 0x40005
                           // vec_a = [1,2,3,4,5,6,7,8]
    for (i, val) in [1u32, 2, 3, 4, 5, 6, 7, 8].iter().enumerate() {
        // ADDI x1, x0, val
        code.push((((*val as u32) & 0xFFF) << 20) | (0 << 15) | (0 << 12) | (1 << 7) | 0x13);
        // SW x1, offset(x20)
        let off = (i * 4) as u32;
        let imm_hi = (off >> 5) & 0x7F;
        let imm_lo = off & 0x1F;
        code.push((imm_hi << 25) | (1 << 20) | (20 << 15) | (2 << 12) | (imm_lo << 7) | 0x23);
    }

    // ── Write vec_b to VRAM+0x5000 (bus 0x4000_6000) ──
    // x21 = 0x4000_6000
    code.push(0x40006AB7); // LUI x21, 0x40006
    for (i, val) in [10u32, 20, 30, 40, 50, 60, 70, 80].iter().enumerate() {
        // ADDI x1, x0, val
        code.push((((*val as u32) & 0xFFF) << 20) | (0 << 15) | (0 << 12) | (1 << 7) | 0x13);
        // SW x1, offset(x21)
        let off = (i * 4) as u32;
        let imm_hi = (off >> 5) & 0x7F;
        let imm_lo = off & 0x1F;
        code.push((imm_hi << 25) | (1 << 20) | (21 << 15) | (2 << 12) | (imm_lo << 7) | 0x23);
    }

    // ── Write kernel code to VRAM+0x0000 (bus 0x4000_1000) ──
    // The kernel computes: result[global_tid] = vec_a[global_tid] + vec_b[global_tid]
    // Using VRAM-local addresses: vec_a@0x4000, vec_b@0x5000, result@0x6000
    // a0=tid, a1=block_id, a2=block_dim, a3=grid_dim
    let kernel: [u32; 14] = [
        0x02C580B3, // MUL  x1, x11, x12     ; block_id * block_dim
        0x00A080B3, // ADD  x1, x1, x10      ; global_tid
        0x00209113, // SLLI x2, x1, 2         ; byte offset
        0x000042B7, // LUI  x5, 0x4
        0x002282B3, // ADD  x5, x5, x2
        0x0002A183, // LW   x3, 0(x5)         ; vec_a[gtid]
        0x00005337, // LUI  x6, 0x5
        0x00230333, // ADD  x6, x6, x2
        0x00032203, // LW   x4, 0(x6)         ; vec_b[gtid]
        0x004183B3, // ADD  x7, x3, x4
        0x00006437, // LUI  x8, 0x6
        0x00240433, // ADD  x8, x8, x2
        0x00742023, // SW   x7, 0(x8)         ; result[gtid]
        0x00000073, // ECALL
    ];

    // x22 = 0x4000_1000 (VRAM base on bus)
    code.push(0x40001B37); // LUI x22, 0x40001
    for (i, &insn) in kernel.iter().enumerate() {
        // Load kernel instruction into x1
        // LUI x1, upper20; ADDI/ORI x1, lower12 — need a general approach
        // Use LUI + ADDI for each 32-bit value
        let upper = insn & 0xFFFFF000;
        let lower = ((insn as i32) << 20 >> 20) as u32; // sign-extended lower 12

        // If lower is negative (bit 11 set), LUI needs +1 adjustment
        let adj_upper = if lower & 0x800 != 0 {
            upper.wrapping_add(0x1000)
        } else {
            upper
        };

        // LUI x1, adj_upper >> 12
        code.push(adj_upper | (1 << 7) | 0x37);
        // ADDI x1, x1, lower (sign-extended 12-bit)
        let imm12 = (insn as i32 as i64) & 0xFFF;
        code.push(((imm12 as u32 & 0xFFF) << 20) | (1 << 15) | (0 << 12) | (1 << 7) | 0x13);
        // SW x1, offset(x22)
        let off = (i * 4) as u32;
        let imm_hi = (off >> 5) & 0x7F;
        let imm_lo = off & 0x1F;
        code.push((imm_hi << 25) | (1 << 20) | (22 << 15) | (2 << 12) | (imm_lo << 7) | 0x23);
    }

    // ── Configure GPU and dispatch kernel ──
    // x23 = 0x4000_0000 (GPU CTRL base)
    code.push(0x40000BB7); // LUI x23, 0x40000

    // OP = 3 (KERNEL) → CTRL+0x08
    code.push(0x00300093); // ADDI x1, x0, 3
    let sw_off8 = (0u32 << 25) | (1 << 20) | (23 << 15) | (2 << 12) | (8 << 7) | 0x23;
    code.push(sw_off8); // SW x1, 8(x23)

    // KERN_ADDR = 0 → CTRL+0x1C
    // SW x0, 28(x23): imm=28=0b11100, imm_hi=0, imm_lo=28
    let sw_off28 = (0u32 << 25) | (0 << 20) | (23 << 15) | (2 << 12) | (28 << 7) | 0x23;
    code.push(sw_off28);

    // GRID_DIM = 2 → CTRL+0x20
    code.push(0x00200093); // ADDI x1, x0, 2
                           // SW x1, 32(x23): imm=32=0b100000, imm_hi=1, imm_lo=0
    let sw_off32 = (1u32 << 25) | (1 << 20) | (23 << 15) | (2 << 12) | (0 << 7) | 0x23;
    code.push(sw_off32);

    // BLOCK_DIM = 4 → CTRL+0x24
    code.push(0x00400093); // ADDI x1, x0, 4
                           // SW x1, 36(x23): imm=36=0b100100, imm_hi=1, imm_lo=4
    let sw_off36 = (1u32 << 25) | (1 << 20) | (23 << 15) | (2 << 12) | (4 << 7) | 0x23;
    code.push(sw_off36);

    // DOORBELL = 1 → CTRL+0x00
    code.push(0x00100093); // ADDI x1, x0, 1
    let sw_off0 = (0u32 << 25) | (1 << 20) | (23 << 15) | (2 << 12) | (0 << 7) | 0x23;
    code.push(sw_off0); // SW x1, 0(x23)

    // ── Read results and sum ──
    // Results are at VRAM+0x6000 (bus 0x4000_7000)
    // x24 = 0x4000_7000
    code.push(0x40007C37); // LUI x24, 0x40007

    // Sum 8 values into x25
    // x25 = 0 (accumulator)
    code.push(0x00000C93); // ADDI x25, x0, 0
    for i in 0u32..8 {
        let off = i * 4;
        // LW x1, off(x24)
        code.push(((off & 0xFFF) << 20) | (24 << 15) | (2 << 12) | (1 << 7) | 0x03);
        // ADD x25, x25, x1
        code.push((0 << 25) | (1 << 20) | (25 << 15) | (0 << 12) | (25 << 7) | 0x33);
    }

    // Store sum to DRAM 0x200
    // x26 = 0x200
    code.push(0x20000D13); // ADDI x26, x0, 0x200
                           // SD x25, 0(x26)
    code.push((0u32 << 25) | (25 << 20) | (26 << 15) | (3 << 12) | (0 << 7) | 0x23);

    // ECALL (halt)
    code.push(0x00000073);

    code
}

#[test]
fn cpu_dispatches_kernel_to_gpgpu() {
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

    // Load host program into DRAM
    let host_code = build_host_program();
    for (i, &insn) in host_code.iter().enumerate() {
        bus.write(
            DRAM_BASE + (i * 4) as u64,
            MemAccessSize::U32,
            insn as u64,
            0,
        )
        .unwrap();
    }

    // ── Phase 1: RV host sets up GPU, dispatches kernel ──
    let mut rv = RiscvCpu::with_bus("rv-host", bus.clone(), DRAM_BASE);
    rv.exec(2000).unwrap();
    assert!(rv.is_halted(), "RV host should halt after setup");

    // Verify GPU has pending command
    assert!(gpu.ctrl().has_pending(), "GPU should have pending kernel");

    // Verify kernel code was written to VRAM
    assert_eq!(gpu_vram.read_u32(0).unwrap(), 0x02C580B3); // first kernel insn

    // Verify input vectors
    assert_eq!(gpu_vram.read_u32(0x4000).unwrap(), 1);
    assert_eq!(gpu_vram.read_u32(0x5000).unwrap(), 10);

    // ── Phase 2: GPU SIMT-executes the kernel ──
    let mut gpu = gpu;
    gpu.exec(10000).unwrap();

    // Verify results in VRAM+0x6000: [11, 22, 33, 44, 55, 66, 77, 88]
    let expected: [u32; 8] = [11, 22, 33, 44, 55, 66, 77, 88];
    for (i, &exp) in expected.iter().enumerate() {
        let val = gpu_vram.read_u32(0x6000 + (i as u64) * 4).unwrap();
        assert_eq!(val, exp, "VRAM result mismatch at index {i}");
    }

    // ── Phase 3: Verify host CPU read-back ──
    // The RV host already read the results (before GPU executed — but in real
    // usage the host would wait for GPU completion). Since we're running
    // sequentially, the host read stale zeros. That's expected in this test
    // structure. The key verification is the VRAM results above.
    //
    // For a proper end-to-end, let's re-run a minimal RV program that reads
    // the now-computed results and sums them.

    let reader_code: Vec<u32> = vec![
        0x40007C37, // LUI x24, 0x40007     ; x24 = 0x4000_7000 (VRAM+0x6000 on bus)
        0x00000C93, // ADDI x25, x0, 0      ; accumulator
        // Load and sum 8 values
        0x000C2083, // LW x1, 0(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x004C2083, // LW x1, 4(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x008C2083, // LW x1, 8(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x00CC2083, // LW x1, 12(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x010C2083, // LW x1, 16(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x014C2083, // LW x1, 20(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x018C2083, // LW x1, 24(x24)
        0x001C8CB3, // ADD x25, x25, x1
        0x01CC2083, // LW x1, 28(x24)
        0x001C8CB3, // ADD x25, x25, x1
        // Store sum to DRAM 0x200
        0x20000D13, // ADDI x26, x0, 0x200
        0x019D3023, // SD x25, 0(x26)
        0x00000073, // ECALL
    ];

    let reader_base = 0xC000u64;
    for (i, &insn) in reader_code.iter().enumerate() {
        bus.write(
            reader_base + (i * 4) as u64,
            MemAccessSize::U32,
            insn as u64,
            0,
        )
        .unwrap();
    }

    let mut rv2 = RiscvCpu::with_bus("rv-reader", bus.clone(), reader_base);
    rv2.exec(100).unwrap();
    assert!(rv2.is_halted(), "Reader should halt");

    // Verify: sum = 11+22+33+44+55+66+77+88 = 396
    let sum = bus.read(0x200, MemAccessSize::U64, 0).unwrap();
    assert_eq!(sum, 396, "Kernel result sum: 11+22+33+44+55+66+77+88 = 396");
}
