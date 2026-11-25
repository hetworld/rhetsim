//! Streaming Multiprocessor — SIMT execution with RV64I hart array.

use std::sync::Arc;
use rhetsim_cpu_riscv::decode::{decode, Instruction};
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use crate::vram_ctrl::GpgpuVram;

pub const DEFAULT_WARP_SIZE: usize = 4;
pub const DEFAULT_NUM_SMS: usize = 2;

pub struct SimtHart {
    regs: [u64; 32],
    pc: u64,
    halted: bool,
}

impl SimtHart {
    pub fn new(pc: u64, thread_id: u32, block_id: u32, block_dim: u32, grid_dim: u32) -> Self {
        let mut hart = Self {
            regs: [0; 32],
            pc,
            halted: false,
        };
        hart.regs[10] = thread_id as u64;
        hart.regs[11] = block_id as u64;
        hart.regs[12] = block_dim as u64;
        hart.regs[13] = grid_dim as u64;
        hart
    }

    pub fn is_halted(&self) -> bool { self.halted }

    fn set_rd(&mut self, rd: u8, val: u64) {
        if rd != 0 {
            self.regs[rd as usize] = val;
        }
    }
}

pub struct Warp {
    threads: Vec<SimtHart>,
}

impl Warp {
    pub fn new(threads: Vec<SimtHart>) -> Self {
        Self { threads }
    }

    pub fn is_complete(&self) -> bool {
        self.threads.iter().all(|t| t.halted)
    }
}

pub struct StreamingMultiprocessor {
    pub id: u32,
    warps: Vec<Warp>,
    vram: Arc<GpgpuVram>,
    current_warp: usize,
}

impl StreamingMultiprocessor {
    pub fn new(id: u32, vram: Arc<GpgpuVram>) -> Self {
        Self { id, warps: Vec::new(), vram, current_warp: 0 }
    }

    pub fn add_warp(&mut self, warp: Warp) {
        self.warps.push(warp);
    }

    pub fn is_done(&self) -> bool {
        self.warps.is_empty() || self.warps.iter().all(|w| w.is_complete())
    }

    pub fn step(&mut self) -> u64 {
        if self.warps.is_empty() || self.is_done() {
            return 0;
        }

        let num_warps = self.warps.len();
        for _ in 0..num_warps {
            let idx = self.current_warp % num_warps;
            self.current_warp = (self.current_warp + 1) % num_warps;

            if !self.warps[idx].is_complete() {
                for t in 0..self.warps[idx].threads.len() {
                    if !self.warps[idx].threads[t].halted {
                        Self::step_thread(&self.vram, &mut self.warps[idx].threads[t]);
                    }
                }
                return 1;
            }
        }
        0
    }

    fn step_thread(vram: &GpgpuVram, hart: &mut SimtHart) {
        let raw = match vram.read(hart.pc, MemAccessSize::U32) {
            Ok(val) => val as u32,
            Err(_) => { hart.halted = true; return; }
        };

        if raw == 0 {
            hart.halted = true;
            return;
        }

        let insn = decode(raw);
        Self::execute(vram, hart, insn);
        hart.regs[0] = 0;
    }

    fn vram_read(vram: &GpgpuVram, addr: u64, size: usize) -> u64 {
        let access = match size {
            1 => MemAccessSize::U8,
            2 => MemAccessSize::U16,
            4 => MemAccessSize::U32,
            8 => MemAccessSize::U64,
            _ => MemAccessSize::U8,
        };
        vram.read(addr, access).unwrap_or(0)
    }

    fn vram_write(vram: &GpgpuVram, addr: u64, size: usize, val: u64) {
        let access = match size {
            1 => MemAccessSize::U8,
            2 => MemAccessSize::U16,
            4 => MemAccessSize::U32,
            8 => MemAccessSize::U64,
            _ => MemAccessSize::U8,
        };
        let _ = vram.write(addr, access, val);
    }

    fn execute(vram: &GpgpuVram, hart: &mut SimtHart, insn: Instruction) {
        match insn {
            Instruction::Lui { rd, imm } => {
                hart.set_rd(rd, imm as u64);
                hart.pc += 4;
            }
            Instruction::Auipc { rd, imm } => {
                hart.set_rd(rd, hart.pc.wrapping_add(imm as u64));
                hart.pc += 4;
            }
            Instruction::Jal { rd, offset } => {
                hart.set_rd(rd, hart.pc + 4);
                hart.pc = hart.pc.wrapping_add(offset as u64);
            }
            Instruction::Jalr { rd, rs1, offset } => {
                let target = (hart.regs[rs1 as usize].wrapping_add(offset as u64)) & !1;
                hart.set_rd(rd, hart.pc + 4);
                hart.pc = target;
            }
            Instruction::Branch { funct3, rs1, rs2, offset } => {
                let a = hart.regs[rs1 as usize];
                let b = hart.regs[rs2 as usize];
                let taken = match funct3 {
                    0 => a == b,
                    1 => a != b,
                    4 => (a as i64) < (b as i64),
                    5 => (a as i64) >= (b as i64),
                    6 => a < b,
                    7 => a >= b,
                    _ => false,
                };
                if taken {
                    hart.pc = hart.pc.wrapping_add(offset as u64);
                } else {
                    hart.pc += 4;
                }
            }
            Instruction::Load { funct3, rd, rs1, offset } => {
                let addr = hart.regs[rs1 as usize].wrapping_add(offset as u64);
                let val = match funct3 {
                    0 => Self::vram_read(vram, addr, 1) as i8 as i64 as u64,
                    1 => Self::vram_read(vram, addr, 2) as i16 as i64 as u64,
                    2 => Self::vram_read(vram, addr, 4) as i32 as i64 as u64,
                    3 => Self::vram_read(vram, addr, 8),
                    4 => Self::vram_read(vram, addr, 1),
                    5 => Self::vram_read(vram, addr, 2),
                    6 => Self::vram_read(vram, addr, 4),
                    _ => 0,
                };
                hart.set_rd(rd, val);
                hart.pc += 4;
            }
            Instruction::Store { funct3, rs1, rs2, offset } => {
                let addr = hart.regs[rs1 as usize].wrapping_add(offset as u64);
                let val = hart.regs[rs2 as usize];
                match funct3 {
                    0 => Self::vram_write(vram, addr, 1, val),
                    1 => Self::vram_write(vram, addr, 2, val),
                    2 => Self::vram_write(vram, addr, 4, val),
                    3 => Self::vram_write(vram, addr, 8, val),
                    _ => {}
                }
                hart.pc += 4;
            }
            Instruction::OpImm { funct3, rd, rs1, imm, funct7 } => {
                let src = hart.regs[rs1 as usize];
                let shamt = (imm as u64) & 0x3F;
                let val = match funct3 {
                    0 => src.wrapping_add(imm as u64),
                    1 => src << shamt,
                    2 => ((src as i64) < (imm as i64)) as u64,
                    3 => (src < (imm as u64)) as u64,
                    4 => src ^ (imm as u64),
                    5 => {
                        if funct7 & 0x20 != 0 {
                            ((src as i64) >> shamt) as u64
                        } else {
                            src >> shamt
                        }
                    }
                    6 => src | (imm as u64),
                    7 => src & (imm as u64),
                    _ => 0,
                };
                hart.set_rd(rd, val);
                hart.pc += 4;
            }
            Instruction::Op { funct3, funct7, rd, rs1, rs2 } => {
                let a = hart.regs[rs1 as usize];
                let b = hart.regs[rs2 as usize];
                let val = if funct7 == 0x01 {
                    match funct3 {
                        0 => a.wrapping_mul(b),
                        1 => ((a as i64 as i128).wrapping_mul(b as i64 as i128) >> 64) as u64,
                        2 => {
                            let sa = a as i64 as i128;
                            let ub = b as u128 as i128;
                            (sa.wrapping_mul(ub) >> 64) as u64
                        }
                        3 => ((a as u128).wrapping_mul(b as u128) >> 64) as u64,
                        4 => {
                            let (sa, sb) = (a as i64, b as i64);
                            if sb == 0 { u64::MAX }
                            else if sa == i64::MIN && sb == -1 { sa as u64 }
                            else { sa.wrapping_div(sb) as u64 }
                        }
                        5 => if b == 0 { u64::MAX } else { a / b },
                        6 => {
                            let (sa, sb) = (a as i64, b as i64);
                            if sb == 0 { a }
                            else if sa == i64::MIN && sb == -1 { 0 }
                            else { sa.wrapping_rem(sb) as u64 }
                        }
                        7 => if b == 0 { a } else { a % b },
                        _ => 0,
                    }
                } else {
                    match (funct3, funct7) {
                        (0, 0x00) => a.wrapping_add(b),
                        (0, 0x20) => a.wrapping_sub(b),
                        (1, 0x00) => a << (b & 0x3F),
                        (2, 0x00) => ((a as i64) < (b as i64)) as u64,
                        (3, 0x00) => (a < b) as u64,
                        (4, 0x00) => a ^ b,
                        (5, 0x00) => a >> (b & 0x3F),
                        (5, 0x20) => ((a as i64) >> (b & 0x3F)) as u64,
                        (6, 0x00) => a | b,
                        (7, 0x00) => a & b,
                        _ => 0,
                    }
                };
                hart.set_rd(rd, val);
                hart.pc += 4;
            }
            Instruction::OpImm32 { funct3, rd, rs1, imm, funct7 } => {
                let src = hart.regs[rs1 as usize] as u32;
                let shamt = (imm as u32) & 0x1F;
                let val = match funct3 {
                    0 => src.wrapping_add(imm as u32),
                    1 => src << shamt,
                    5 => {
                        if funct7 & 0x20 != 0 {
                            ((src as i32) >> shamt) as u32
                        } else {
                            src >> shamt
                        }
                    }
                    _ => 0,
                };
                hart.set_rd(rd, val as i32 as i64 as u64);
                hart.pc += 4;
            }
            Instruction::Op32 { funct3, funct7, rd, rs1, rs2 } => {
                let a = hart.regs[rs1 as usize] as u32;
                let b = hart.regs[rs2 as usize] as u32;
                let val = if funct7 == 0x01 {
                    match funct3 {
                        0 => a.wrapping_mul(b),
                        4 => {
                            let (sa, sb) = (a as i32, b as i32);
                            if sb == 0 { u32::MAX }
                            else if sa == i32::MIN && sb == -1 { sa as u32 }
                            else { sa.wrapping_div(sb) as u32 }
                        }
                        5 => if b == 0 { u32::MAX } else { a / b },
                        6 => {
                            let (sa, sb) = (a as i32, b as i32);
                            if sb == 0 { a }
                            else if sa == i32::MIN && sb == -1 { 0 }
                            else { sa.wrapping_rem(sb) as u32 }
                        }
                        7 => if b == 0 { a } else { a % b },
                        _ => 0,
                    }
                } else {
                    match (funct3, funct7) {
                        (0, 0x00) => a.wrapping_add(b),
                        (0, 0x20) => a.wrapping_sub(b),
                        (1, 0x00) => a << (b & 0x1F),
                        (5, 0x00) => a >> (b & 0x1F),
                        (5, 0x20) => ((a as i32) >> (b & 0x1F)) as u32,
                        _ => 0,
                    }
                };
                hart.set_rd(rd, val as i32 as i64 as u64);
                hart.pc += 4;
            }
            Instruction::Fence => { hart.pc += 4; }
            Instruction::Ecall | Instruction::Ebreak => { hart.halted = true; }
            _ => { hart.halted = true; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_insn(vram: &GpgpuVram, offset: u64, insn: u32) {
        vram.write(offset, MemAccessSize::U32, insn as u64).unwrap();
    }

    #[test]
    fn test_single_thread_addi() {
        let vram = Arc::new(GpgpuVram::new(4096));

        // ADDI x1, x0, 42; ECALL
        write_insn(&vram, 0, 0x02A00093);
        write_insn(&vram, 4, 0x00000073);

        let hart = SimtHart::new(0, 0, 0, 1, 1);
        let warp = Warp::new(vec![hart]);

        let mut sm = StreamingMultiprocessor::new(0, vram);
        sm.add_warp(warp);

        while !sm.is_done() {
            sm.step();
        }

        assert!(sm.warps[0].threads[0].is_halted());
        assert_eq!(sm.warps[0].threads[0].regs[1], 42);
    }

    #[test]
    fn test_simt_parallel_add() {
        let vram = Arc::new(GpgpuVram::new(0x10000));

        // Write input data: vec_a at 0x4000, vec_b at 0x5000
        let vec_a: [u32; 4] = [10, 20, 30, 40];
        let vec_b: [u32; 4] = [1, 2, 3, 4];
        for (i, &v) in vec_a.iter().enumerate() {
            vram.write_u32(0x4000 + (i as u64) * 4, v).unwrap();
        }
        for (i, &v) in vec_b.iter().enumerate() {
            vram.write_u32(0x5000 + (i as u64) * 4, v).unwrap();
        }

        // Kernel: result[tid] = vec_a[tid] + vec_b[tid]
        // a0=tid, a1=block_id, a2=block_dim, a3=grid_dim
        let kernel: Vec<u32> = vec![
            0x00251093, // SLLI x1, x10, 2       ; x1 = tid * 4
            0x000042B7, // LUI  x5, 0x4           ; x5 = 0x4000
            0x001282B3, // ADD  x5, x5, x1        ; x5 = &vec_a[tid]
            0x0002A103, // LW   x2, 0(x5)         ; x2 = vec_a[tid]
            0x00005337, // LUI  x6, 0x5           ; x6 = 0x5000
            0x00130333, // ADD  x6, x6, x1        ; x6 = &vec_b[tid]
            0x00032183, // LW   x3, 0(x6)         ; x3 = vec_b[tid]
            0x00310233, // ADD  x4, x2, x3        ; x4 = a + b
            0x000063B7, // LUI  x7, 0x6           ; x7 = 0x6000
            0x001383B3, // ADD  x7, x7, x1        ; x7 = &result[tid]
            0x0043A023, // SW   x4, 0(x7)         ; result[tid] = sum
            0x00000073, // ECALL
        ];
        for (i, &insn) in kernel.iter().enumerate() {
            write_insn(&vram, (i * 4) as u64, insn);
        }

        // Create 4 threads, 1 warp
        let threads: Vec<SimtHart> = (0..4)
            .map(|tid| SimtHart::new(0, tid, 0, 4, 1))
            .collect();
        let warp = Warp::new(threads);

        let mut sm = StreamingMultiprocessor::new(0, vram.clone());
        sm.add_warp(warp);

        let mut steps = 0u64;
        while !sm.is_done() {
            sm.step();
            steps += 1;
            assert!(steps < 1000, "SM didn't converge");
        }

        // Verify results at 0x6000
        let expected: [u32; 4] = [11, 22, 33, 44];
        for (i, &exp) in expected.iter().enumerate() {
            let val = vram.read_u32(0x6000 + (i as u64) * 4).unwrap();
            assert_eq!(val, exp, "mismatch at thread {}", i);
        }
    }

    #[test]
    fn test_warp_round_robin() {
        let vram = Arc::new(GpgpuVram::new(4096));

        // Simple kernel: store tid to result[tid]
        // SLLI x1, x10, 2; LUI x5, 0; ADD x5, x5, x1; SW x10, 512(x5); ECALL
        //
        // Store to offset 512 + tid*4
        let kernel: Vec<u32> = vec![
            0x00251093, // SLLI x1, x10, 2
            0x20051023, // SH... no, let me use SW with offset
            // Actually let's make it simpler: SW x10, 0(x1) where x1 = 0x200 + tid*4
            // ADDI x1, x0, 0x200; SLLI x2, x10, 2; ADD x1, x1, x2; SW x10, 0(x1); ECALL
        ];

        // Let me re-encode:
        // ADDI x1, x0, 0x200: 0x20000093
        // SLLI x2, x10, 2:    0x00251113
        // ADD  x1, x1, x2:    0x002080B3
        // SW   x10, 0(x1):    Store funct3=2, rs1=1, rs2=10
        //   = (0 << 25) | (10 << 20) | (1 << 15) | (2 << 12) | (0 << 7) | 0x23
        //   = 0x00A0A023
        // ECALL: 0x00000073
        let kernel: Vec<u32> = vec![
            0x20000093, // ADDI x1, x0, 0x200
            0x00251113, // SLLI x2, x10, 2
            0x002080B3, // ADD  x1, x1, x2
            0x00A0A023, // SW   x10, 0(x1)
            0x00000073, // ECALL
        ];
        for (i, &insn) in kernel.iter().enumerate() {
            write_insn(&vram, (i * 4) as u64, insn);
        }

        // 2 warps of 2 threads each
        let warp0 = Warp::new(vec![
            SimtHart::new(0, 0, 0, 4, 1),
            SimtHart::new(0, 1, 0, 4, 1),
        ]);
        let warp1 = Warp::new(vec![
            SimtHart::new(0, 2, 0, 4, 1),
            SimtHart::new(0, 3, 0, 4, 1),
        ]);

        let mut sm = StreamingMultiprocessor::new(0, vram.clone());
        sm.add_warp(warp0);
        sm.add_warp(warp1);

        let mut steps = 0u64;
        while !sm.is_done() {
            sm.step();
            steps += 1;
            assert!(steps < 1000);
        }

        // All 4 threads wrote their tid to 0x200 + tid*4
        for tid in 0u32..4 {
            let val = vram.read_u32(0x200 + (tid as u64) * 4).unwrap();
            assert_eq!(val, tid, "thread {} didn't write correctly", tid);
        }
    }

    #[test]
    fn test_multi_block_kernel() {
        let vram = Arc::new(GpgpuVram::new(0x10000));

        // Kernel: global_tid = block_id * block_dim + thread_id
        //         result[global_tid] = global_tid * 10
        //
        // a0=tid, a1=block_id, a2=block_dim
        // MUL x1, x11, x12          ; x1 = block_id * block_dim
        // ADD x1, x1, x10           ; x1 = global_tid
        // ADDI x2, x0, 10           ; x2 = 10
        // MUL x3, x1, x2            ; x3 = global_tid * 10
        // SLLI x4, x1, 2            ; x4 = global_tid * 4
        // LUI x5, 0x8               ; x5 = 0x8000
        // ADD x5, x5, x4            ; x5 = &result[global_tid]
        // SW x3, 0(x5)              ; store
        // ECALL
        let mul_x1_x11_x12 = (0x01 << 25) | (12 << 20) | (11 << 15) | (0 << 12) | (1 << 7) | 0x33;
        let add_x1_x1_x10 = (0 << 25) | (10 << 20) | (1 << 15) | (0 << 12) | (1 << 7) | 0x33;
        let mul_x3_x1_x2 = (0x01 << 25) | (2 << 20) | (1 << 15) | (0 << 12) | (3 << 7) | 0x33;
        let sw_x3_0_x5 = (0 << 25) | (3 << 20) | (5 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let kernel: Vec<u32> = vec![
            mul_x1_x11_x12,  // MUL  x1, x11, x12
            add_x1_x1_x10,   // ADD  x1, x1, x10
            0x00A00113,       // ADDI x2, x0, 10
            mul_x3_x1_x2,    // MUL  x3, x1, x2
            0x00209213,       // SLLI x4, x1, 2
            0x000082B7,       // LUI  x5, 0x8
            0x004282B3,       // ADD  x5, x5, x4
            sw_x3_0_x5,       // SW   x3, 0(x5)
            0x00000073,       // ECALL
        ];
        for (i, &insn) in kernel.iter().enumerate() {
            write_insn(&vram, (i * 4) as u64, insn);
        }

        // 2 blocks × 2 threads, warp_size = 2
        let mut sm = StreamingMultiprocessor::new(0, vram.clone());
        for block_id in 0u32..2 {
            let threads: Vec<SimtHart> = (0..2)
                .map(|tid| SimtHart::new(0, tid, block_id, 2, 2))
                .collect();
            sm.add_warp(Warp::new(threads));
        }

        let mut steps = 0u64;
        while !sm.is_done() {
            sm.step();
            steps += 1;
            assert!(steps < 1000);
        }

        // global_tid 0,1,2,3 → values 0,10,20,30
        for gtid in 0u32..4 {
            let val = vram.read_u32(0x8000 + (gtid as u64) * 4).unwrap();
            assert_eq!(val, gtid * 10, "global thread {} wrong", gtid);
        }
    }
}
