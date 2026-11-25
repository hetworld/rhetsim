//! RiscvCpu — RV64IMA interpreter with M-mode CSR and trap delivery

use std::sync::Arc;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::MemAccessSize;
use crate::csr::{self, CsrFile};
use crate::decode::{decode, Instruction};

enum MemBackend {
    Flat {
        memory: Vec<u8>,
        uart_output: Vec<u8>,
        uart_addr: u64,
    },
    Bus {
        bus: Arc<MemBus>,
    },
}

pub struct RiscvCpu {
    id: UnitId,
    regs: [u64; 32],
    pc: u64,
    halted: bool,
    mem_base: u64,
    insn_count: u64,
    backend: MemBackend,
    csr: CsrFile,
    reservation: Option<u64>,
}

impl RiscvCpu {
    pub fn new(id: &str, mem_size: usize, mem_base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            regs: [0; 32],
            pc: mem_base,
            halted: false,
            mem_base,
            insn_count: 0,
            backend: MemBackend::Flat {
                memory: vec![0u8; mem_size],
                uart_output: Vec::new(),
                uart_addr: 0,
            },
            csr: CsrFile::new(0),
            reservation: None,
        }
    }

    pub fn with_bus(id: &str, bus: Arc<MemBus>, mem_base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            regs: [0; 32],
            pc: mem_base,
            halted: false,
            mem_base,
            insn_count: 0,
            backend: MemBackend::Bus { bus },
            csr: CsrFile::new(0),
            reservation: None,
        }
    }

    pub fn load_binary(&mut self, data: &[u8], offset: u64) {
        match &mut self.backend {
            MemBackend::Flat { memory, .. } => {
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

    pub fn set_uart_addr(&mut self, addr: u64) {
        if let MemBackend::Flat { uart_addr, .. } = &mut self.backend {
            *uart_addr = addr;
        }
    }

    pub fn uart_output(&self) -> &[u8] {
        match &self.backend {
            MemBackend::Flat { uart_output, .. } => uart_output,
            MemBackend::Bus { .. } => &[],
        }
    }

    pub fn pc(&self) -> u64 { self.pc }
    pub fn reg(&self, idx: usize) -> u64 { self.regs[idx] }
    pub fn is_halted(&self) -> bool { self.halted }
    pub fn insn_count(&self) -> u64 { self.insn_count }
    pub fn csr(&self) -> &CsrFile { &self.csr }
    pub fn csr_mut(&mut self) -> &mut CsrFile { &mut self.csr }

    fn mem_read(&self, addr: u64, size: usize) -> u64 {
        match &self.backend {
            MemBackend::Flat { memory, uart_addr, .. } => {
                if *uart_addr != 0 && addr == *uart_addr + 5 {
                    return 0x60;
                }
                let mem_end = self.mem_base + memory.len() as u64;
                if addr >= self.mem_base && addr < mem_end {
                    let off = (addr - self.mem_base) as usize;
                    let end = (off + size).min(memory.len());
                    let mut val = 0u64;
                    for i in 0..(end - off) {
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
            MemBackend::Flat { memory, uart_output, uart_addr } => {
                if *uart_addr != 0 && addr == *uart_addr {
                    uart_output.push(val as u8);
                    return;
                }
                let mem_end = self.mem_base + memory.len() as u64;
                if addr >= self.mem_base && addr < mem_end {
                    let off = (addr - self.mem_base) as usize;
                    let end = (off + size).min(memory.len());
                    for i in 0..(end - off) {
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

    fn check_interrupt(&mut self) -> bool {
        if let Some(cause) = self.csr.pending_interrupt() {
            let target = self.csr.trap_enter(self.pc, csr::MCAUSE_INTERRUPT | cause, 0);
            self.pc = target;
            true
        } else {
            false
        }
    }

    fn step(&mut self) {
        self.check_interrupt();

        let raw = self.mem_read(self.pc, 4) as u32;
        if raw == 0 {
            self.halted = true;
            return;
        }
        let insn = decode(raw);
        self.execute(insn);
        self.regs[0] = 0;
        self.insn_count += 1;
    }

    fn set_rd(&mut self, rd: u8, val: u64) {
        if rd != 0 {
            self.regs[rd as usize] = val;
        }
    }

    fn execute(&mut self, insn: Instruction) {
        match insn {
            Instruction::Lui { rd, imm } => {
                self.set_rd(rd, imm as u64);
                self.pc += 4;
            }
            Instruction::Auipc { rd, imm } => {
                self.set_rd(rd, self.pc.wrapping_add(imm as u64));
                self.pc += 4;
            }
            Instruction::Jal { rd, offset } => {
                self.set_rd(rd, self.pc + 4);
                self.pc = self.pc.wrapping_add(offset as u64);
            }
            Instruction::Jalr { rd, rs1, offset } => {
                let target = (self.regs[rs1 as usize].wrapping_add(offset as u64)) & !1;
                self.set_rd(rd, self.pc + 4);
                self.pc = target;
            }
            Instruction::Branch { funct3, rs1, rs2, offset } => {
                let a = self.regs[rs1 as usize];
                let b = self.regs[rs2 as usize];
                let taken = match funct3 {
                    0 => a == b,                            // BEQ
                    1 => a != b,                            // BNE
                    4 => (a as i64) < (b as i64),           // BLT
                    5 => (a as i64) >= (b as i64),          // BGE
                    6 => a < b,                             // BLTU
                    7 => a >= b,                            // BGEU
                    _ => false,
                };
                if taken {
                    self.pc = self.pc.wrapping_add(offset as u64);
                } else {
                    self.pc += 4;
                }
            }
            Instruction::Load { funct3, rd, rs1, offset } => {
                let addr = self.regs[rs1 as usize].wrapping_add(offset as u64);
                let val = match funct3 {
                    0 => self.mem_read(addr, 1) as i8 as i64 as u64,   // LB
                    1 => self.mem_read(addr, 2) as i16 as i64 as u64,  // LH
                    2 => self.mem_read(addr, 4) as i32 as i64 as u64,  // LW
                    3 => self.mem_read(addr, 8),                        // LD
                    4 => self.mem_read(addr, 1),                        // LBU
                    5 => self.mem_read(addr, 2),                        // LHU
                    6 => self.mem_read(addr, 4),                        // LWU
                    _ => 0,
                };
                self.set_rd(rd, val);
                self.pc += 4;
            }
            Instruction::Store { funct3, rs1, rs2, offset } => {
                let addr = self.regs[rs1 as usize].wrapping_add(offset as u64);
                let val = self.regs[rs2 as usize];
                match funct3 {
                    0 => self.mem_write(addr, 1, val), // SB
                    1 => self.mem_write(addr, 2, val), // SH
                    2 => self.mem_write(addr, 4, val), // SW
                    3 => self.mem_write(addr, 8, val), // SD
                    _ => {}
                }
                self.pc += 4;
            }
            Instruction::OpImm { funct3, rd, rs1, imm, funct7 } => {
                let src = self.regs[rs1 as usize];
                let shamt = (imm as u64) & 0x3F;
                let val = match funct3 {
                    0 => src.wrapping_add(imm as u64),               // ADDI
                    1 => src << shamt,                                // SLLI
                    2 => ((src as i64) < (imm as i64)) as u64,       // SLTI
                    3 => (src < (imm as u64)) as u64,                // SLTIU
                    4 => src ^ (imm as u64),                         // XORI
                    5 => {
                        if funct7 & 0x20 != 0 {
                            ((src as i64) >> shamt) as u64            // SRAI
                        } else {
                            src >> shamt                               // SRLI
                        }
                    }
                    6 => src | (imm as u64),                         // ORI
                    7 => src & (imm as u64),                         // ANDI
                    _ => 0,
                };
                self.set_rd(rd, val);
                self.pc += 4;
            }
            Instruction::Op { funct3, funct7, rd, rs1, rs2 } => {
                let a = self.regs[rs1 as usize];
                let b = self.regs[rs2 as usize];
                let val = if funct7 == 0x01 {
                    // M extension
                    self.exec_m(funct3, a, b)
                } else {
                    match (funct3, funct7) {
                        (0, 0x00) => a.wrapping_add(b),                  // ADD
                        (0, 0x20) => a.wrapping_sub(b),                  // SUB
                        (1, 0x00) => a << (b & 0x3F),                    // SLL
                        (2, 0x00) => ((a as i64) < (b as i64)) as u64,   // SLT
                        (3, 0x00) => (a < b) as u64,                     // SLTU
                        (4, 0x00) => a ^ b,                              // XOR
                        (5, 0x00) => a >> (b & 0x3F),                    // SRL
                        (5, 0x20) => ((a as i64) >> (b & 0x3F)) as u64,  // SRA
                        (6, 0x00) => a | b,                              // OR
                        (7, 0x00) => a & b,                              // AND
                        _ => 0,
                    }
                };
                self.set_rd(rd, val);
                self.pc += 4;
            }
            Instruction::OpImm32 { funct3, rd, rs1, imm, funct7 } => {
                let src = self.regs[rs1 as usize] as u32;
                let shamt = (imm as u32) & 0x1F;
                let val = match funct3 {
                    0 => src.wrapping_add(imm as u32),               // ADDIW
                    1 => src << shamt,                                // SLLIW
                    5 => {
                        if funct7 & 0x20 != 0 {
                            ((src as i32) >> shamt) as u32            // SRAIW
                        } else {
                            src >> shamt                               // SRLIW
                        }
                    }
                    _ => 0,
                };
                self.set_rd(rd, val as i32 as i64 as u64);
                self.pc += 4;
            }
            Instruction::Op32 { funct3, funct7, rd, rs1, rs2 } => {
                let a = self.regs[rs1 as usize] as u32;
                let b = self.regs[rs2 as usize] as u32;
                let val = if funct7 == 0x01 {
                    // M extension (W variants)
                    self.exec_m32(funct3, a, b)
                } else {
                    match (funct3, funct7) {
                        (0, 0x00) => a.wrapping_add(b),                  // ADDW
                        (0, 0x20) => a.wrapping_sub(b),                  // SUBW
                        (1, 0x00) => a << (b & 0x1F),                    // SLLW
                        (5, 0x00) => a >> (b & 0x1F),                    // SRLW
                        (5, 0x20) => ((a as i32) >> (b & 0x1F)) as u32,  // SRAW
                        _ => 0,
                    }
                };
                self.set_rd(rd, val as i32 as i64 as u64);
                self.pc += 4;
            }
            Instruction::Csr { funct3, rd, rs1, csr } => {
                let old = self.csr.read(csr, self.insn_count).unwrap_or(0);
                match funct3 {
                    1 => { // CSRRW
                        self.set_rd(rd, old);
                        self.csr.write(csr, self.regs[rs1 as usize]);
                    }
                    2 => { // CSRRS
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old | self.regs[rs1 as usize]);
                        }
                    }
                    3 => { // CSRRC
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old & !self.regs[rs1 as usize]);
                        }
                    }
                    5 => { // CSRRWI
                        self.set_rd(rd, old);
                        self.csr.write(csr, rs1 as u64);
                    }
                    6 => { // CSRRSI
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old | rs1 as u64);
                        }
                    }
                    7 => { // CSRRCI
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old & !(rs1 as u64));
                        }
                    }
                    _ => {}
                }
                self.pc += 4;
            }
            Instruction::Mret => {
                self.pc = self.csr.mret();
            }
            Instruction::Wfi => {
                self.pc += 4;
            }
            Instruction::Amo { funct5, aq: _, rl: _, funct3, rd, rs1, rs2 } => {
                self.exec_amo(funct5, funct3, rd, rs1, rs2);
                self.pc += 4;
            }
            Instruction::Ecall | Instruction::Ebreak => {
                self.halted = true;
            }
            Instruction::Fence => {
                self.pc += 4;
            }
            Instruction::Unknown(_) => {
                self.halted = true;
            }
        }
    }

    // RV64M: 64-bit multiply/divide
    fn exec_m(&self, funct3: u8, a: u64, b: u64) -> u64 {
        match funct3 {
            0 => a.wrapping_mul(b),                                           // MUL
            1 => ((a as i64 as i128).wrapping_mul(b as i64 as i128) >> 64) as u64, // MULH
            2 => {                                                             // MULHSU
                let sa = a as i64 as i128;
                let ub = b as u128 as i128;
                (sa.wrapping_mul(ub) >> 64) as u64
            }
            3 => ((a as u128).wrapping_mul(b as u128) >> 64) as u64,          // MULHU
            4 => {                                                             // DIV
                let sa = a as i64;
                let sb = b as i64;
                if sb == 0 { u64::MAX }
                else if sa == i64::MIN && sb == -1 { sa as u64 }
                else { sa.wrapping_div(sb) as u64 }
            }
            5 => {                                                             // DIVU
                if b == 0 { u64::MAX } else { a / b }
            }
            6 => {                                                             // REM
                let sa = a as i64;
                let sb = b as i64;
                if sb == 0 { a }
                else if sa == i64::MIN && sb == -1 { 0 }
                else { sa.wrapping_rem(sb) as u64 }
            }
            7 => {                                                             // REMU
                if b == 0 { a } else { a % b }
            }
            _ => 0,
        }
    }

    // RV64M: 32-bit multiply/divide (W variants)
    fn exec_m32(&self, funct3: u8, a: u32, b: u32) -> u32 {
        match funct3 {
            0 => a.wrapping_mul(b),                                           // MULW
            4 => {                                                             // DIVW
                let sa = a as i32;
                let sb = b as i32;
                if sb == 0 { u32::MAX }
                else if sa == i32::MIN && sb == -1 { sa as u32 }
                else { sa.wrapping_div(sb) as u32 }
            }
            5 => {                                                             // DIVUW
                if b == 0 { u32::MAX } else { a / b }
            }
            6 => {                                                             // REMW
                let sa = a as i32;
                let sb = b as i32;
                if sb == 0 { a }
                else if sa == i32::MIN && sb == -1 { 0 }
                else { sa.wrapping_rem(sb) as u32 }
            }
            7 => {                                                             // REMUW
                if b == 0 { a } else { a % b }
            }
            _ => 0,
        }
    }

    // RV64A: atomic memory operations
    fn exec_amo(&mut self, funct5: u8, funct3: u8, rd: u8, rs1: u8, rs2: u8) {
        let addr = self.regs[rs1 as usize];
        let src2 = self.regs[rs2 as usize];
        let is_word = funct3 == 2; // 010 = W, 011 = D

        match funct5 {
            0x02 => { // LR
                let val = if is_word {
                    self.mem_read(addr, 4) as i32 as i64 as u64
                } else {
                    self.mem_read(addr, 8)
                };
                self.set_rd(rd, val);
                self.reservation = Some(addr);
            }
            0x03 => { // SC
                if self.reservation == Some(addr) {
                    if is_word {
                        self.mem_write(addr, 4, src2);
                    } else {
                        self.mem_write(addr, 8, src2);
                    }
                    self.set_rd(rd, 0); // success
                } else {
                    self.set_rd(rd, 1); // failure
                }
                self.reservation = None;
            }
            _ => {
                // AMO operations: load old, compute, store new, write old to rd
                let (old, new_val) = if is_word {
                    let old = self.mem_read(addr, 4) as i32 as i64 as u64;
                    let s2 = src2 as i32;
                    let o = old as i32;
                    let nv = match funct5 {
                        0x01 => src2 as u32,                                       // AMOSWAP
                        0x00 => o.wrapping_add(s2) as u32,                         // AMOADD
                        0x04 => (old as u32) ^ (src2 as u32),                      // AMOXOR
                        0x0C => (old as u32) & (src2 as u32),                      // AMOAND
                        0x08 => (old as u32) | (src2 as u32),                      // AMOOR
                        0x10 => o.min(s2) as u32,                                  // AMOMIN
                        0x14 => o.max(s2) as u32,                                  // AMOMAX
                        0x18 => (old as u32).min(src2 as u32),                     // AMOMINU
                        0x1C => (old as u32).max(src2 as u32),                     // AMOMAXU
                        _ => old as u32,
                    };
                    (old, nv as i32 as i64 as u64)
                } else {
                    let old = self.mem_read(addr, 8);
                    let nv = match funct5 {
                        0x01 => src2,                                              // AMOSWAP
                        0x00 => (old as i64).wrapping_add(src2 as i64) as u64,     // AMOADD
                        0x04 => old ^ src2,                                        // AMOXOR
                        0x0C => old & src2,                                        // AMOAND
                        0x08 => old | src2,                                        // AMOOR
                        0x10 => (old as i64).min(src2 as i64) as u64,              // AMOMIN
                        0x14 => (old as i64).max(src2 as i64) as u64,              // AMOMAX
                        0x18 => old.min(src2),                                     // AMOMINU
                        0x1C => old.max(src2),                                     // AMOMAXU
                        _ => old,
                    };
                    (old, nv)
                };
                if is_word {
                    self.mem_write(addr, 4, new_val);
                } else {
                    self.mem_write(addr, 8, new_val);
                }
                self.set_rd(rd, old);
            }
        }
    }
}

impl Unit for RiscvCpu {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "riscv-rv64-core" }
    fn caps(&self) -> UnitCaps { UnitCaps::EXECUTABLE }

    fn reset(&mut self) -> OmniResult<()> {
        self.regs = [0; 32];
        self.pc = self.mem_base;
        self.halted = false;
        self.insn_count = 0;
        self.csr.reset();
        self.reservation = None;
        if let MemBackend::Flat { uart_output, .. } = &mut self.backend {
            uart_output.clear();
        }
        Ok(())
    }

    fn execution_backend(&mut self) -> Option<&mut dyn ExecutionBackend> {
        Some(self)
    }
}

impl ExecutionBackend for RiscvCpu {
    fn exec(&mut self, budget_ns: u64) -> OmniResult<u64> {
        let mut executed = 0u64;
        while executed < budget_ns && !self.halted {
            self.step();
            executed += 1;
        }
        Ok(executed)
    }

    fn reset(&mut self) -> OmniResult<()> {
        Unit::reset(self)
    }

    fn save_state(&self) -> OmniResult<Vec<u8>> {
        let csr_data = self.csr.save();
        let mut state = Vec::with_capacity(32 * 8 + 8 + 1 + csr_data.len());
        for r in &self.regs {
            state.extend_from_slice(&r.to_le_bytes());
        }
        state.extend_from_slice(&self.pc.to_le_bytes());
        state.push(self.halted as u8);
        state.extend_from_slice(&csr_data);
        Ok(state)
    }

    fn load_state(&mut self, data: &[u8]) -> OmniResult<()> {
        let min_size = 32 * 8 + 8 + 1;
        if data.len() < min_size {
            return Err(OmniError::SnapshotError("riscv state too short".into()));
        }
        for i in 0..32 {
            let off = i * 8;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&data[off..off + 8]);
            self.regs[i] = u64::from_le_bytes(buf);
        }
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&data[256..264]);
        self.pc = u64::from_le_bytes(buf);
        self.halted = data[264] != 0;
        if data.len() >= min_size + CsrFile::state_size() {
            self.csr.load(&data[min_size..]);
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str { "rv64ima-interp" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_with(insns: &[u32]) -> RiscvCpu {
        let mut cpu = RiscvCpu::new("test", 4096, 0x8000_0000);
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu
    }

    #[test]
    fn test_addi() {
        let mut cpu = cpu_with(&[
            0x02A00093, // ADDI x1, x0, 42
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 42);
        assert!(cpu.is_halted());
    }

    #[test]
    fn test_add_sub() {
        let mut cpu = cpu_with(&[
            0x00A00093, // ADDI x1, x0, 10
            0x01400113, // ADDI x2, x0, 20
            0x002081B3, // ADD x3, x1, x2
            0x40208233, // SUB x4, x1, x2
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 30);
        assert_eq!(cpu.reg(4) as i64, -10);
    }

    #[test]
    fn test_lui() {
        let mut cpu = cpu_with(&[
            0x123450B7, // LUI x1, 0x12345
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 0x12345000);
    }

    #[test]
    fn test_store_load() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        let insns: Vec<u32> = vec![
            0x02A00093, // ADDI x1, x0, 42
            0x10000113, // ADDI x2, x0, 256
            0x00112023, // SW x1, 0(x2)
            0x00012183, // LW x3, 0(x2)
            0x00000073, // ECALL
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 42);
        assert_eq!(cpu.reg(3), 42);
    }

    #[test]
    fn test_branch_taken() {
        let mut cpu = cpu_with(&[
            0x00A00093, // ADDI x1, x0, 10
            0x00A00113, // ADDI x2, x0, 10
            0x00208463, // BEQ x1, x2, +8
            0x06400093, // ADDI x1, x0, 100
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 10);
    }

    #[test]
    fn test_branch_not_taken() {
        let mut cpu = cpu_with(&[
            0x00A00093, // ADDI x1, x0, 10
            0x01400113, // ADDI x2, x0, 20
            0x00208463, // BEQ x1, x2, +8
            0x06400093, // ADDI x1, x0, 100
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 100);
    }

    #[test]
    fn test_jal() {
        let mut cpu = cpu_with(&[
            0x008000EF, // JAL x1, +8
            0x06400093, // ADDI x1, x0, 100
            0x00000073, // ECALL
        ]);
        let base = cpu.pc();
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), base + 4);
    }

    #[test]
    fn test_uart_output() {
        let uart_addr = 0x1000_0000u64;
        let mut cpu = RiscvCpu::new("test", 4096, 0x8000_0000);
        cpu.set_uart_addr(uart_addr);
        let insns: Vec<u32> = vec![
            0x10000137, // LUI x2, 0x10000
            0x04800093, // ADDI x1, x0, 72
            0x00110023, // SB x1, 0(x2)
            0x00000073, // ECALL
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.exec(10).unwrap();
        assert_eq!(cpu.uart_output(), b"H");
    }

    #[test]
    fn test_save_load_state() {
        let mut cpu = cpu_with(&[
            0x02A00093, // ADDI x1, x0, 42
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        let state = cpu.save_state().unwrap();
        let mut cpu2 = RiscvCpu::new("test2", 4096, 0x8000_0000);
        cpu2.load_state(&state).unwrap();
        assert_eq!(cpu2.reg(1), 42);
        assert!(cpu2.is_halted());
    }

    #[test]
    fn test_loop_countdown() {
        let mut cpu = cpu_with(&[
            0x00500093, // ADDI x1, x0, 5
            0xFFF08093, // ADDI x1, x1, -1
            0xFE009EE3, // BNE x1, x0, -4
            0x00000073, // ECALL
        ]);
        cpu.exec(100).unwrap();
        assert_eq!(cpu.reg(1), 0);
        assert!(cpu.is_halted());
    }

    #[test]
    fn test_x0_always_zero() {
        let mut cpu = cpu_with(&[
            0x02A00013, // ADDI x0, x0, 42
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn test_bus_backed_read_write() {
        use rhetsim_core::mem::MemAttrs;

        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram0", 64 * 1024, 0));
        let mut bus = MemBus::new("sys-bus");
        bus.map(dram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
                0, MemAttrs::rw(), None).unwrap();
        let bus = Arc::new(bus);

        let mut cpu = RiscvCpu::with_bus("rv-core0", bus.clone(), 0);

        let insns: Vec<u32> = vec![
            0x06300093, // ADDI x1, x0, 99
            0x10000113, // ADDI x2, x0, 256
            0x00112023, // SW x1, 0(x2)
            0x00012183, // LW x3, 0(x2)
            0x00000073, // ECALL
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(10).unwrap();

        assert_eq!(cpu.reg(1), 99);
        assert_eq!(cpu.reg(3), 99);
        assert!(cpu.is_halted());

        let val = bus.read(0x100, MemAccessSize::U32, 0).unwrap();
        assert_eq!(val, 99);
    }

    // ── CSR tests ──────────────────────────────────────────────

    #[test]
    fn test_csrrw() {
        // CSRRW x1, mscratch, x2: write x2 to mscratch, old mscratch to x1
        // mscratch = 0x340
        // [31:20]=0x340, [19:15]=rs1=2, [14:12]=001, [11:7]=rd=1, [6:0]=0x73
        let csrrw = (0x340 << 20) | (2 << 15) | (1 << 12) | (1 << 7) | 0x73;
        let mut cpu = cpu_with(&[
            0x06400113, // ADDI x2, x0, 100
            csrrw,
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 0); // old mscratch was 0
        assert_eq!(cpu.csr().mscratch, 100);
    }

    #[test]
    fn test_csrrs_mie() {
        // Enable machine timer interrupt via CSRRS mie, x1
        // mie = 0x304
        // Set x1 = MIP_MTIP (1<<7 = 128)
        let csrrs = (0x304 << 20) | (1 << 15) | (2 << 12) | (2 << 7) | 0x73;
        let mut cpu = cpu_with(&[
            0x08000093, // ADDI x1, x0, 128  (MIP_MTIP)
            csrrs,       // CSRRS x2, mie, x1
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.csr().mie, 128);
    }

    // ── M extension tests ──────────────────────────────────────

    #[test]
    fn test_mul() {
        // MUL x3, x1, x2: funct7=0x01, funct3=0, opcode=0x33
        let mul = (0x01 << 25) | (2 << 20) | (1 << 15) | (0 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x00700093, // ADDI x1, x0, 7
            0x00600113, // ADDI x2, x0, 6
            mul,
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 42);
    }

    #[test]
    fn test_div() {
        // DIV x3, x1, x2: funct7=0x01, funct3=4, opcode=0x33
        let div = (0x01 << 25) | (2 << 20) | (1 << 15) | (4 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x02A00093, // ADDI x1, x0, 42
            0x00700113, // ADDI x2, x0, 7
            div,
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 6);
    }

    #[test]
    fn test_div_by_zero() {
        let div = (0x01 << 25) | (2 << 20) | (1 << 15) | (4 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x02A00093, // ADDI x1, x0, 42
            0x00000113, // ADDI x2, x0, 0
            div,
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), u64::MAX); // -1 per spec
    }

    #[test]
    fn test_rem() {
        // REM x3, x1, x2: funct7=0x01, funct3=6, opcode=0x33
        let rem = (0x01 << 25) | (2 << 20) | (1 << 15) | (6 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x02B00093, // ADDI x1, x0, 43
            0x00700113, // ADDI x2, x0, 7
            rem,
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 1); // 43 % 7 = 1
    }

    // ── A extension tests ──────────────────────────────────────

    #[test]
    fn test_lr_sc_w() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        // Store 42 at addr 0x100, then LR.W + SC.W
        // ADDI x1, x0, 42;  ADDI x2, x0, 0x100;  SW x1, 0(x2)
        // LR.W x3, (x2);  ADDI x4, x0, 99;  SC.W x5, x4, (x2)
        // LW x6, 0(x2);  ECALL
        let sw     = (0 << 25) | (1 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let lr_w   = (0x02 << 27) | (0 << 20) | (2 << 15) | (2 << 12) | (3 << 7) | 0x2F;
        let sc_w   = (0x03 << 27) | (4 << 20) | (2 << 15) | (2 << 12) | (5 << 7) | 0x2F;
        let lw_x6  = (0 << 20) | (2 << 15) | (2 << 12) | (6 << 7) | 0x03;
        let insns: Vec<u32> = vec![
            0x02A00093, // ADDI x1, x0, 42
            0x10000113, // ADDI x2, x0, 256
            sw,          // SW x1, 0(x2)
            lr_w,        // LR.W x3, (x2)
            0x06300213, // ADDI x4, x0, 99
            sc_w,        // SC.W x5, x4, (x2)
            lw_x6,       // LW x6, 0(x2)
            0x00000073, // ECALL
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(20).unwrap();
        assert_eq!(cpu.reg(3), 42); // LR loaded 42
        assert_eq!(cpu.reg(5), 0);  // SC succeeded
        assert_eq!(cpu.reg(6), 99); // memory updated to 99
    }

    #[test]
    fn test_amoadd_w() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        // Store 10 at addr 0x100, then AMOADD.W x3, x4, (x2)
        let sw       = (0 << 25) | (1 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let amoadd_w = (0x00 << 27) | (4 << 20) | (2 << 15) | (2 << 12) | (3 << 7) | 0x2F;
        let lw_x5    = (0 << 20) | (2 << 15) | (2 << 12) | (5 << 7) | 0x03;
        let insns: Vec<u32> = vec![
            0x00A00093, // ADDI x1, x0, 10
            0x10000113, // ADDI x2, x0, 256
            sw,          // SW x1, 0(x2)
            0x00500213, // ADDI x4, x0, 5
            amoadd_w,    // AMOADD.W x3, x4, (x2)
            lw_x5,       // LW x5, 0(x2)
            0x00000073, // ECALL
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(20).unwrap();
        assert_eq!(cpu.reg(3), 10); // old value
        assert_eq!(cpu.reg(5), 15); // 10 + 5
    }

    // ── Trap delivery test ─────────────────────────────────────

    #[test]
    fn test_trap_delivery_and_mret() {
        // Set up:
        // 1. Set mtvec to trap handler address
        // 2. Enable interrupts (mstatus.MIE, mie.MTIE)
        // 3. Force mip.MTIP pending
        // 4. Next step should trap
        // 5. Handler sets x10=1 and does MRET
        let mut cpu = RiscvCpu::new("test", 8192, 0);

        // Main code at 0x0000:
        //   ADDI x1, x0, 0x100       ; x1 = handler addr
        //   CSRRW x0, mtvec, x1      ; mtvec = 0x100
        //   ADDI x2, x0, 128         ; x2 = 1<<7 = MTIE
        //   CSRRS x0, mie, x2        ; enable MTIE
        //   ADDI x3, x0, 8           ; x3 = 1<<3 = MIE
        //   CSRRS x0, mstatus, x3    ; enable global MIE
        //   ADDI x11, x0, 42         ; x11 = 42 (should execute after MRET)
        //   ECALL
        let csrrw_mtvec    = (0x305 << 20) | (1 << 15) | (1 << 12) | (0 << 7) | 0x73;
        let csrrs_mie      = (0x304 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x73;
        let csrrs_mstatus  = (0x300 << 20) | (3 << 15) | (2 << 12) | (0 << 7) | 0x73;

        let main_code: Vec<u32> = vec![
            0x10000093, // ADDI x1, x0, 256
            csrrw_mtvec,
            0x08000113, // ADDI x2, x0, 128
            csrrs_mie,
            0x00800193, // ADDI x3, x0, 8
            csrrs_mstatus,
            // At this point, interrupts enabled but no pending interrupt yet.
            // After next step, we set MIP.MTIP externally, and the step after will trap.
            0x02A00593, // ADDI x11, x0, 42
            0x00000073, // ECALL
        ];
        for (i, insn) in main_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }

        // Handler code at 0x100:
        //   ADDI x10, x0, 1          ; x10 = 1 (marker)
        //   ADDI x9, x0, 128         ; x9 = MTIE bit
        //   CSRRC x0, mie, x9        ; clear MTIE to prevent re-trap
        //   MRET
        let mret = 0x30200073u32;
        let csrrc_mie = (0x304u32 << 20) | (9 << 15) | (3 << 12) | (0 << 7) | 0x73;
        let handler: Vec<u32> = vec![
            0x00100513, // ADDI x10, x0, 1
            0x08000493, // ADDI x9, x0, 128
            csrrc_mie,   // CSRRC x0, mie, x9
            mret,
        ];
        for (i, insn) in handler.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x100 + i * 4) as u64);
        }

        cpu.set_pc(0);

        // Execute up to and including the CSRRS mstatus (6 instructions)
        cpu.exec(6).unwrap();
        assert!(!cpu.is_halted());

        // Now set MTIP pending
        cpu.csr_mut().mip |= csr::MIP_MTIP;

        // Next step should detect interrupt and jump to handler
        cpu.exec(1).unwrap();
        // We should now be in the handler or just after trap entry
        // The trap handler writes x10=1
        // We need a few more steps to complete handler + MRET + continue
        cpu.exec(10).unwrap();

        assert_eq!(cpu.reg(10), 1);  // handler executed
        assert_eq!(cpu.reg(11), 42); // returned from handler, continued execution
    }
}
