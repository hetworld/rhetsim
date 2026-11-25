//! AArch64 CPU — core integer interpreter

use std::sync::Arc;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::MemAccessSize;
use crate::decode::{decode, Instruction, ShiftType, Cond, WriteBack};

enum MemBackend {
    Flat { memory: Vec<u8> },
    Bus { bus: Arc<MemBus> },
}

pub struct ArmCpu {
    id: UnitId,
    regs: [u64; 31], // X0-X30
    sp: u64,
    pc: u64,
    nzcv: u32,       // bits [31:28] = N,Z,C,V
    halted: bool,
    mem_base: u64,
    insn_count: u64,
    backend: MemBackend,
}

impl ArmCpu {
    pub fn new(id: &str, mem_size: usize, mem_base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            regs: [0; 31],
            sp: mem_base + mem_size as u64,
            pc: mem_base,
            nzcv: 0,
            halted: false,
            mem_base,
            insn_count: 0,
            backend: MemBackend::Flat { memory: vec![0u8; mem_size] },
        }
    }

    pub fn with_bus(id: &str, bus: Arc<MemBus>, mem_base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            regs: [0; 31],
            sp: 0,
            pc: mem_base,
            nzcv: 0,
            halted: false,
            mem_base,
            insn_count: 0,
            backend: MemBackend::Bus { bus },
        }
    }

    pub fn load_binary(&mut self, data: &[u8], offset: u64) {
        match &mut self.backend {
            MemBackend::Flat { memory } => {
                let start = offset as usize;
                let end = start + data.len();
                if end <= memory.len() {
                    memory[start..end].copy_from_slice(data);
                }
            }
            MemBackend::Bus { bus } => {
                let addr = self.mem_base + offset;
                for (i, &byte) in data.iter().enumerate() {
                    let _ = bus.write(addr + i as u64, MemAccessSize::U8, byte as u64, 0);
                }
            }
        }
    }

    pub fn set_pc(&mut self, pc: u64) { self.pc = pc; }
    pub fn set_sp(&mut self, sp: u64) { self.sp = sp; }
    pub fn pc(&self) -> u64 { self.pc }
    pub fn sp(&self) -> u64 { self.sp }
    pub fn reg(&self, idx: usize) -> u64 { self.regs[idx] }
    pub fn is_halted(&self) -> bool { self.halted }
    pub fn insn_count(&self) -> u64 { self.insn_count }

    fn read_gpr(&self, r: u8) -> u64 {
        if r == 31 { 0 } else { self.regs[r as usize] }
    }

    fn read_gpr_or_sp(&self, r: u8) -> u64 {
        if r == 31 { self.sp } else { self.regs[r as usize] }
    }

    fn write_gpr(&mut self, r: u8, val: u64) {
        if r < 31 { self.regs[r as usize] = val; }
    }

    fn write_gpr_or_sp(&mut self, r: u8, val: u64) {
        if r == 31 { self.sp = val; } else { self.regs[r as usize] = val; }
    }

    fn mem_read(&self, addr: u64, size: usize) -> u64 {
        match &self.backend {
            MemBackend::Flat { memory } => {
                let mem_end = self.mem_base + memory.len() as u64;
                if addr >= self.mem_base && addr + size as u64 <= mem_end {
                    let off = (addr - self.mem_base) as usize;
                    let mut val = 0u64;
                    for i in 0..size {
                        val |= (memory[off + i] as u64) << (i * 8);
                    }
                    val
                } else {
                    0
                }
            }
            MemBackend::Bus { bus } => {
                let access_size = match size {
                    1 => MemAccessSize::U8,
                    2 => MemAccessSize::U16,
                    4 => MemAccessSize::U32,
                    8 => MemAccessSize::U64,
                    _ => MemAccessSize::U8,
                };
                bus.read(addr, access_size, 0).unwrap_or(0)
            }
        }
    }

    fn mem_write(&mut self, addr: u64, size: usize, val: u64) {
        match &mut self.backend {
            MemBackend::Flat { memory } => {
                let mem_end = self.mem_base + memory.len() as u64;
                if addr >= self.mem_base && addr + size as u64 <= mem_end {
                    let off = (addr - self.mem_base) as usize;
                    for i in 0..size {
                        memory[off + i] = (val >> (i * 8)) as u8;
                    }
                }
            }
            MemBackend::Bus { bus } => {
                let access_size = match size {
                    1 => MemAccessSize::U8,
                    2 => MemAccessSize::U16,
                    4 => MemAccessSize::U32,
                    8 => MemAccessSize::U64,
                    _ => MemAccessSize::U8,
                };
                let _ = bus.write(addr, access_size, val, 0);
            }
        }
    }

    fn apply_shift(val: u64, shift: ShiftType, amount: u8, width: u32) -> u64 {
        let amount = amount as u32 % width;
        match shift {
            ShiftType::Lsl => val << amount,
            ShiftType::Lsr => val >> amount,
            ShiftType::Asr => ((val as i64) >> amount) as u64,
            ShiftType::Ror => val.rotate_right(amount),
        }
    }

    fn set_nzcv_add(&mut self, a: u64, b: u64, result: u64, sf: bool) {
        let (n, z, c, v) = if sf {
            let n = (result >> 63) & 1;
            let z = (result == 0) as u64;
            let c = (result < a) as u64;
            let v = (((a ^ !b) & (a ^ result)) >> 63) & 1;
            (n, z, c, v)
        } else {
            let a32 = a as u32;
            let b32 = b as u32;
            let r32 = result as u32;
            let n = ((r32 >> 31) & 1) as u64;
            let z = (r32 == 0) as u64;
            let c = ((a32 as u64) + (b32 as u64) > 0xFFFF_FFFF) as u64;
            let v = ((((a32 ^ !b32) & (a32 ^ r32)) >> 31) & 1) as u64;
            (n, z, c, v)
        };
        self.nzcv = ((n << 3 | z << 2 | c << 1 | v) << 28) as u32;
    }

    fn set_nzcv_sub(&mut self, a: u64, b: u64, result: u64, sf: bool) {
        let (n, z, c, v) = if sf {
            let n = (result >> 63) & 1;
            let z = (result == 0) as u64;
            let c = (a >= b) as u64;
            let v = (((a ^ b) & (a ^ result)) >> 63) & 1;
            (n, z, c, v)
        } else {
            let a32 = a as u32;
            let b32 = b as u32;
            let r32 = result as u32;
            let n = ((r32 >> 31) & 1) as u64;
            let z = (r32 == 0) as u64;
            let c = (a32 >= b32) as u64;
            let v = ((((a32 ^ b32) & (a32 ^ r32)) >> 31) & 1) as u64;
            (n, z, c, v)
        };
        self.nzcv = ((n << 3 | z << 2 | c << 1 | v) << 28) as u32;
    }

    fn set_nzcv_logic(&mut self, result: u64, sf: bool) {
        let (n, z) = if sf {
            (((result >> 63) & 1) as u64, (result == 0) as u64)
        } else {
            ((((result as u32) >> 31) & 1) as u64, (result as u32 == 0) as u64)
        };
        self.nzcv = ((n << 3 | z << 2) << 28) as u32;
    }

    fn cond_holds(&self, cond: Cond) -> bool {
        let n = (self.nzcv >> 31) & 1 != 0;
        let z = (self.nzcv >> 30) & 1 != 0;
        let c = (self.nzcv >> 29) & 1 != 0;
        let v = (self.nzcv >> 28) & 1 != 0;
        match cond {
            Cond::Eq => z,
            Cond::Ne => !z,
            Cond::Cs => c,
            Cond::Cc => !c,
            Cond::Mi => n,
            Cond::Pl => !n,
            Cond::Vs => v,
            Cond::Vc => !v,
            Cond::Hi => c && !z,
            Cond::Ls => !c || z,
            Cond::Ge => n == v,
            Cond::Lt => n != v,
            Cond::Gt => !z && n == v,
            Cond::Le => z || n != v,
            Cond::Al | Cond::Nv => true,
        }
    }

    fn trunc32(val: u64) -> u64 {
        val & 0xFFFF_FFFF
    }

    fn step(&mut self) {
        let raw = self.mem_read(self.pc, 4) as u32;
        if raw == 0 {
            self.halted = true;
            return;
        }
        let insn = decode(raw);
        self.execute(insn);
        self.insn_count += 1;
    }

    fn execute(&mut self, insn: Instruction) {
        match insn {
            // ── Data processing immediate ──────────────────────
            Instruction::AddImm { sf, rd, rn, imm, shift } => {
                let a = self.read_gpr_or_sp(rn);
                let b = if shift { (imm as u64) << 12 } else { imm as u64 };
                let result = if sf { a.wrapping_add(b) } else { Self::trunc32(a.wrapping_add(b)) };
                self.write_gpr_or_sp(rd, result);
                self.pc += 4;
            }
            Instruction::SubImm { sf, rd, rn, imm, shift } => {
                let a = self.read_gpr_or_sp(rn);
                let b = if shift { (imm as u64) << 12 } else { imm as u64 };
                let result = if sf { a.wrapping_sub(b) } else { Self::trunc32(a.wrapping_sub(b)) };
                self.write_gpr_or_sp(rd, result);
                self.pc += 4;
            }
            Instruction::AddsImm { sf, rd, rn, imm, shift } => {
                let a = self.read_gpr_or_sp(rn);
                let b = if shift { (imm as u64) << 12 } else { imm as u64 };
                let result = if sf { a.wrapping_add(b) } else { Self::trunc32(a.wrapping_add(b)) };
                self.set_nzcv_add(a, b, result, sf);
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::SubsImm { sf, rd, rn, imm, shift } => {
                let a = self.read_gpr_or_sp(rn);
                let b = if shift { (imm as u64) << 12 } else { imm as u64 };
                let result = if sf { a.wrapping_sub(b) } else { Self::trunc32(a.wrapping_sub(b)) };
                self.set_nzcv_sub(a, b, result, sf);
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::MovZ { sf, rd, imm, hw } => {
                let val = (imm as u64) << (hw as u64 * 16);
                let val = if sf { val } else { Self::trunc32(val) };
                self.write_gpr(rd, val);
                self.pc += 4;
            }
            Instruction::MovK { sf: _, rd, imm, hw } => {
                let shift = hw as u64 * 16;
                let mask = 0xFFFFu64 << shift;
                let old = self.read_gpr(rd);
                let val = (old & !mask) | ((imm as u64) << shift);
                self.write_gpr(rd, val);
                self.pc += 4;
            }
            Instruction::MovN { sf, rd, imm, hw } => {
                let val = !((imm as u64) << (hw as u64 * 16));
                let val = if sf { val } else { Self::trunc32(val) };
                self.write_gpr(rd, val);
                self.pc += 4;
            }
            // ── Logical immediate ──────────────────────────────
            Instruction::AndImm { sf, rd, rn, imm } => {
                let result = self.read_gpr_or_sp(rn) & imm;
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr_or_sp(rd, result);
                self.pc += 4;
            }
            Instruction::OrrImm { sf, rd, rn, imm } => {
                let result = self.read_gpr_or_sp(rn) | imm;
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr_or_sp(rd, result);
                self.pc += 4;
            }
            Instruction::EorImm { sf, rd, rn, imm } => {
                let result = self.read_gpr_or_sp(rn) ^ imm;
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr_or_sp(rd, result);
                self.pc += 4;
            }
            Instruction::AndsImm { sf, rd, rn, imm } => {
                let result = self.read_gpr(rn) & imm;
                let result = if sf { result } else { Self::trunc32(result) };
                self.set_nzcv_logic(result, sf);
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            // ── Data processing register ───────────────────────
            Instruction::AddReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let a = self.read_gpr(rn);
                let b = Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { a.wrapping_add(b) } else { Self::trunc32(a.wrapping_add(b)) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::SubReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let a = self.read_gpr(rn);
                let b = Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { a.wrapping_sub(b) } else { Self::trunc32(a.wrapping_sub(b)) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::AddsReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let a = self.read_gpr(rn);
                let b = Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { a.wrapping_add(b) } else { Self::trunc32(a.wrapping_add(b)) };
                self.set_nzcv_add(a, b, result, sf);
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::SubsReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let a = self.read_gpr(rn);
                let b = Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { a.wrapping_sub(b) } else { Self::trunc32(a.wrapping_sub(b)) };
                self.set_nzcv_sub(a, b, result, sf);
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::AndReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let result = self.read_gpr(rn) & Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::OrrReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let result = self.read_gpr(rn) | Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::EorReg { sf, rd, rn, rm, shift, amount } => {
                let width = if sf { 64 } else { 32 };
                let result = self.read_gpr(rn) ^ Self::apply_shift(self.read_gpr(rm), shift, amount, width);
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            // ── Multiply/Divide ────────────────────────────────
            Instruction::Madd { sf, rd, rn, rm, ra } => {
                let a = self.read_gpr(rn);
                let b = self.read_gpr(rm);
                let c = self.read_gpr(ra);
                let result = c.wrapping_add(a.wrapping_mul(b));
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::Msub { sf, rd, rn, rm, ra } => {
                let a = self.read_gpr(rn);
                let b = self.read_gpr(rm);
                let c = self.read_gpr(ra);
                let result = c.wrapping_sub(a.wrapping_mul(b));
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::Sdiv { sf, rd, rn, rm } => {
                let (a, b) = if sf {
                    (self.read_gpr(rn) as i64, self.read_gpr(rm) as i64)
                } else {
                    (self.read_gpr(rn) as i32 as i64, self.read_gpr(rm) as i32 as i64)
                };
                let result = if b == 0 { 0 }
                    else if a == i64::MIN && b == -1 { a }
                    else { a.wrapping_div(b) };
                let result = if sf { result as u64 } else { Self::trunc32(result as u64) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::Udiv { sf, rd, rn, rm } => {
                let (a, b) = if sf {
                    (self.read_gpr(rn), self.read_gpr(rm))
                } else {
                    (self.read_gpr(rn) & 0xFFFF_FFFF, self.read_gpr(rm) & 0xFFFF_FFFF)
                };
                let result = if b == 0 { 0 } else { a / b };
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            // ── Shifts ─────────────────────────────────────────
            Instruction::Lslv { sf, rd, rn, rm } => {
                let width = if sf { 64u32 } else { 32 };
                let shift = (self.read_gpr(rm) as u32) % width;
                let result = self.read_gpr(rn) << shift;
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::Lsrv { sf, rd, rn, rm } => {
                let width = if sf { 64u32 } else { 32 };
                let shift = (self.read_gpr(rm) as u32) % width;
                let val = if sf { self.read_gpr(rn) } else { self.read_gpr(rn) & 0xFFFF_FFFF };
                let result = val >> shift;
                let result = if sf { result } else { Self::trunc32(result) };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            Instruction::Asrv { sf, rd, rn, rm } => {
                let width = if sf { 64u32 } else { 32 };
                let shift = (self.read_gpr(rm) as u32) % width;
                let result = if sf {
                    ((self.read_gpr(rn) as i64) >> shift) as u64
                } else {
                    Self::trunc32(((self.read_gpr(rn) as i32) >> shift) as u64)
                };
                self.write_gpr(rd, result);
                self.pc += 4;
            }
            // ── Loads and stores ───────────────────────────────
            Instruction::LdrImm { sf, rt, rn, offset, wb } => {
                let size = if sf { 8 } else { 4 };
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb {
                    WriteBack::Pre => base.wrapping_add(offset as u64),
                    _ => base,
                };
                let val = self.mem_read(addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 }), size);
                let val = if !sf { val & 0xFFFF_FFFF } else { val };
                self.write_gpr(rt, val);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::StrImm { sf, rt, rn, offset, wb } => {
                let size = if sf { 8 } else { 4 };
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb {
                    WriteBack::Pre => base.wrapping_add(offset as u64),
                    _ => base,
                };
                let val = self.read_gpr(rt);
                self.mem_write(addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 }), size, val);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::LdrbImm { rt, rn, offset, wb } => {
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                let val = self.mem_read(eff, 1) & 0xFF;
                self.write_gpr(rt, val);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::StrbImm { rt, rn, offset, wb } => {
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                self.mem_write(eff, 1, self.read_gpr(rt) & 0xFF);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::LdrhImm { rt, rn, offset, wb } => {
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                let val = self.mem_read(eff, 2) & 0xFFFF;
                self.write_gpr(rt, val);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::StrhImm { rt, rn, offset, wb } => {
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                self.mem_write(eff, 2, self.read_gpr(rt) & 0xFFFF);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::LdrswImm { rt, rn, offset, wb } => {
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                let val = self.mem_read(eff, 4) as i32 as i64 as u64;
                self.write_gpr(rt, val);
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::LdrLit { sf, rt, offset } => {
                let addr = self.pc.wrapping_add(offset as u64);
                let size = if sf { 8 } else { 4 };
                let val = self.mem_read(addr, size);
                self.write_gpr(rt, val);
                self.pc += 4;
            }
            Instruction::Ldp { sf, rt, rt2, rn, offset, wb } => {
                let size = if sf { 8usize } else { 4 };
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                let v1 = self.mem_read(eff, size);
                let v2 = self.mem_read(eff + size as u64, size);
                self.write_gpr(rt, if sf { v1 } else { v1 & 0xFFFF_FFFF });
                self.write_gpr(rt2, if sf { v2 } else { v2 & 0xFFFF_FFFF });
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            Instruction::Stp { sf, rt, rt2, rn, offset, wb } => {
                let size = if sf { 8usize } else { 4 };
                let base = self.read_gpr_or_sp(rn);
                let addr = match wb { WriteBack::Pre => base.wrapping_add(offset as u64), _ => base };
                let eff = addr.wrapping_add(if wb == WriteBack::None { offset as u64 } else { 0 });
                self.mem_write(eff, size, self.read_gpr(rt));
                self.mem_write(eff + size as u64, size, self.read_gpr(rt2));
                match wb {
                    WriteBack::Pre => self.write_gpr_or_sp(rn, addr),
                    WriteBack::Post => self.write_gpr_or_sp(rn, base.wrapping_add(offset as u64)),
                    WriteBack::None => {}
                }
                self.pc += 4;
            }
            // ── Branches ───────────────────────────────────────
            Instruction::B { offset } => {
                self.pc = self.pc.wrapping_add(offset as u64);
            }
            Instruction::Bl { offset } => {
                self.regs[30] = self.pc + 4; // X30 = LR
                self.pc = self.pc.wrapping_add(offset as u64);
            }
            Instruction::Br { rn } => {
                self.pc = self.read_gpr(rn);
            }
            Instruction::Blr { rn } => {
                let target = self.read_gpr(rn);
                self.regs[30] = self.pc + 4;
                self.pc = target;
            }
            Instruction::Ret { rn } => {
                self.pc = self.read_gpr(rn);
            }
            Instruction::BCond { cond, offset } => {
                if self.cond_holds(cond) {
                    self.pc = self.pc.wrapping_add(offset as u64);
                } else {
                    self.pc += 4;
                }
            }
            Instruction::Cbz { sf, rt, offset } => {
                let val = if sf { self.read_gpr(rt) } else { self.read_gpr(rt) & 0xFFFF_FFFF };
                if val == 0 {
                    self.pc = self.pc.wrapping_add(offset as u64);
                } else {
                    self.pc += 4;
                }
            }
            Instruction::Cbnz { sf, rt, offset } => {
                let val = if sf { self.read_gpr(rt) } else { self.read_gpr(rt) & 0xFFFF_FFFF };
                if val != 0 {
                    self.pc = self.pc.wrapping_add(offset as u64);
                } else {
                    self.pc += 4;
                }
            }
            // ── Address generation ─────────────────────────────
            Instruction::Adr { rd, offset } => {
                self.write_gpr(rd, self.pc.wrapping_add(offset as u64));
                self.pc += 4;
            }
            Instruction::Adrp { rd, offset } => {
                let base = self.pc & !0xFFF;
                self.write_gpr(rd, base.wrapping_add(offset as u64));
                self.pc += 4;
            }
            // ── System ─────────────────────────────────────────
            Instruction::Svc { .. } => {
                self.halted = true;
            }
            Instruction::Mrs { rt, sysreg } => {
                let val = match sysreg {
                    0x5A10 => self.nzcv as u64, // NZCV
                    _ => 0,
                };
                self.write_gpr(rt, val);
                self.pc += 4;
            }
            Instruction::Msr { rt, sysreg } => {
                let val = self.read_gpr(rt);
                match sysreg {
                    0x5A10 => self.nzcv = (val as u32) & 0xF000_0000,
                    _ => {}
                }
                self.pc += 4;
            }
            Instruction::Nop => { self.pc += 4; }
            Instruction::Wfi => { self.pc += 4; }
            Instruction::Unknown(_) => { self.halted = true; }
        }
    }
}

impl Unit for ArmCpu {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "aarch64-core" }
    fn caps(&self) -> UnitCaps { UnitCaps::EXECUTABLE }

    fn reset(&mut self) -> OmniResult<()> {
        self.regs = [0; 31];
        self.sp = 0;
        self.pc = self.mem_base;
        self.nzcv = 0;
        self.halted = false;
        self.insn_count = 0;
        Ok(())
    }

    fn execution_backend(&mut self) -> Option<&mut dyn ExecutionBackend> {
        Some(self)
    }
}

impl ExecutionBackend for ArmCpu {
    fn exec(&mut self, budget_ns: u64) -> OmniResult<u64> {
        let mut executed = 0u64;
        while executed < budget_ns && !self.halted {
            self.step();
            executed += 1;
        }
        Ok(executed)
    }

    fn reset(&mut self) -> OmniResult<()> { Unit::reset(self) }

    fn save_state(&self) -> OmniResult<Vec<u8>> {
        let mut state = Vec::with_capacity(31 * 8 + 8 + 8 + 4 + 1);
        for r in &self.regs { state.extend_from_slice(&r.to_le_bytes()); }
        state.extend_from_slice(&self.sp.to_le_bytes());
        state.extend_from_slice(&self.pc.to_le_bytes());
        state.extend_from_slice(&self.nzcv.to_le_bytes());
        state.push(self.halted as u8);
        Ok(state)
    }

    fn load_state(&mut self, data: &[u8]) -> OmniResult<()> {
        let min = 31 * 8 + 8 + 8 + 4 + 1;
        if data.len() < min {
            return Err(OmniError::SnapshotError("aarch64 state too short".into()));
        }
        for i in 0..31 {
            let off = i * 8;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&data[off..off + 8]);
            self.regs[i] = u64::from_le_bytes(buf);
        }
        let mut buf8 = [0u8; 8];
        buf8.copy_from_slice(&data[248..256]);
        self.sp = u64::from_le_bytes(buf8);
        buf8.copy_from_slice(&data[256..264]);
        self.pc = u64::from_le_bytes(buf8);
        let mut buf4 = [0u8; 4];
        buf4.copy_from_slice(&data[264..268]);
        self.nzcv = u32::from_le_bytes(buf4);
        self.halted = data[268] != 0;
        Ok(())
    }

    fn backend_name(&self) -> &'static str { "aarch64-interp" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_with(insns: &[u32]) -> ArmCpu {
        let mut cpu = ArmCpu::new("test", 65536, 0);
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu
    }

    #[test]
    fn test_movz_add() {
        // MOVZ X0, #10;  MOVZ X1, #20;  ADD X2, X0, X1;  SVC #0
        let mut cpu = cpu_with(&[
            0xD2800140, // MOVZ X0, #10
            0xD2800281, // MOVZ X1, #20
            0x8B010002, // ADD X2, X0, X1
            0xD4000001, // SVC #0
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(2), 30);
        assert!(cpu.is_halted());
    }

    #[test]
    fn test_sub_imm() {
        // MOVZ X0, #100;  SUB X1, X0, #42;  SVC #0
        let mut cpu = cpu_with(&[
            0xD2800C80, // MOVZ X0, #100
            0xD100A801, // SUB X1, X0, #42
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 58);
    }

    #[test]
    fn test_str_ldr() {
        // MOVZ X0, #42;  MOVZ X1, #0x100;  STR X0, [X1];  LDR X2, [X1];  SVC #0
        let mut cpu = cpu_with(&[
            0xD2800540, // MOVZ X0, #42
            0xD2802001, // MOVZ X1, #0x100
            0xF9000020, // STR X0, [X1]
            0xF9400022, // LDR X2, [X1]
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(2), 42);
    }

    #[test]
    fn test_b_cond() {
        // MOVZ X0, #10;  MOVZ X1, #10;  SUBS XZR, X0, X1;  B.EQ +8;  MOVZ X2, #99;  MOVZ X3, #1; SVC #0
        let mut cpu = cpu_with(&[
            0xD2800140, // MOVZ X0, #10
            0xD2800141, // MOVZ X1, #10
            0xEB01001F, // SUBS XZR, X0, X1
            0x54000040, // B.EQ +8 (skip 1 insn, land at MOVZ X3)
            0xD2800C62, // MOVZ X2, #99
            0xD2800023, // MOVZ X3, #1
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(2), 0); // skipped
        assert_eq!(cpu.reg(3), 1); // B.EQ lands here
    }

    #[test]
    fn test_bl_ret() {
        // BL +8;  MOVZ X0, #99;  SVC #0;  <func:> MOVZ X1, #42;  RET
        let mut cpu = cpu_with(&[
            0x94000003, // BL +12 (to insn at offset 12)
            0xD2800C60, // MOVZ X0, #99
            0xD4000001, // SVC #0
            0xD2800541, // MOVZ X1, #42
            0xD65F03C0, // RET
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 42);  // func executed
        assert_eq!(cpu.reg(0), 99);  // returned to after BL
    }

    #[test]
    fn test_cbz() {
        // MOVZ X0, #0;  CBZ X0, +8;  MOVZ X1, #99;  MOVZ X2, #1;  SVC #0
        let mut cpu = cpu_with(&[
            0xD2800000, // MOVZ X0, #0
            0xB4000040, // CBZ X0, +8 (skip 1 insn, land at MOVZ X2)
            0xD2800C61, // MOVZ X1, #99
            0xD2800022, // MOVZ X2, #1
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 0); // skipped
        assert_eq!(cpu.reg(2), 1); // landed here
    }

    #[test]
    fn test_loop_countdown() {
        // MOVZ X0, #5;  SUB X0, X0, #1;  CBNZ X0, -4;  MOVZ X1, #1;  SVC #0
        let mut cpu = cpu_with(&[
            0xD28000A0, // MOVZ X0, #5
            0xD1000400, // SUB X0, X0, #1
            0xB5FFFFE0, // CBNZ X0, -4
            0xD2800021, // MOVZ X1, #1
            0xD4000001,
        ]);
        cpu.exec(100).unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.reg(1), 1);
    }

    #[test]
    fn test_mul() {
        // MOVZ X0, #7;  MOVZ X1, #6;  MUL X2, X0, X1 (= MADD X2,X0,X1,XZR);  SVC #0
        // MADD: 1_00_11011_000_Rm_0_Ra_Rn_Rd, with Ra=XZR(31)
        let madd = 0x9B01_7C02u32; // MADD X2, X0, X1, XZR
        let mut cpu = cpu_with(&[
            0xD28000E0, // MOVZ X0, #7
            0xD28000C1, // MOVZ X1, #6
            madd,
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(2), 42);
    }

    #[test]
    fn test_sdiv() {
        // MOVZ X0, #42;  MOVZ X1, #7;  SDIV X2, X0, X1;  SVC #0
        // SDIV: 1_0_0_11010110_Rm_00001_1_Rn_Rd
        let sdiv = 0x9AC10C02u32; // SDIV X2, X0, X1
        let mut cpu = cpu_with(&[
            0xD2800540, // MOVZ X0, #42
            0xD28000E1, // MOVZ X1, #7
            sdiv,
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(2), 6);
    }

    #[test]
    fn test_stp_ldp() {
        // MOVZ X0, #11;  MOVZ X1, #22;  MOVZ X2, #0x200
        // STP X0, X1, [X2];  LDP X3, X4, [X2];  SVC #0
        let mut cpu = cpu_with(&[
            0xD2800160, // MOVZ X0, #11
            0xD28002C1, // MOVZ X1, #22
            0xD2804002, // MOVZ X2, #0x200
            0xA9000440, // STP X0, X1, [X2]
            0xA9401043, // LDP X3, X4, [X2]
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 11);
        assert_eq!(cpu.reg(4), 22);
    }

    #[test]
    fn test_save_load_state() {
        let mut cpu = cpu_with(&[
            0xD2800540, // MOVZ X0, #42
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        let state = cpu.save_state().unwrap();
        let mut cpu2 = ArmCpu::new("test2", 65536, 0);
        cpu2.load_state(&state).unwrap();
        assert_eq!(cpu2.reg(0), 42);
        assert!(cpu2.is_halted());
    }

    #[test]
    fn test_strb_ldrb() {
        // MOVZ X0, #0xFF;  MOVZ X1, #0x100;  STRB W0, [X1];  LDRB W2, [X1];  SVC #0
        let mut cpu = cpu_with(&[
            0xD2801FE0, // MOVZ X0, #0xFF
            0xD2802001, // MOVZ X1, #0x100
            0x39000020, // STRB W0, [X1]
            0x39400022, // LDRB W2, [X1]
            0xD4000001,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(2), 0xFF);
    }
}
