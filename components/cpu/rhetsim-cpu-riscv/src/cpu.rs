//! RiscvCpu — minimal RV64I interpreter

use std::sync::Arc;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::mem::MemAccessSize;
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

    fn step(&mut self) {
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
                let val = match (funct3, funct7) {
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
                let val = match (funct3, funct7) {
                    (0, 0x00) => a.wrapping_add(b),                  // ADDW
                    (0, 0x20) => a.wrapping_sub(b),                  // SUBW
                    (1, 0x00) => a << (b & 0x1F),                    // SLLW
                    (5, 0x00) => a >> (b & 0x1F),                    // SRLW
                    (5, 0x20) => ((a as i32) >> (b & 0x1F)) as u32,  // SRAW
                    _ => 0,
                };
                self.set_rd(rd, val as i32 as i64 as u64);
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
        let mut state = Vec::with_capacity(32 * 8 + 8 + 1);
        for r in &self.regs {
            state.extend_from_slice(&r.to_le_bytes());
        }
        state.extend_from_slice(&self.pc.to_le_bytes());
        state.push(self.halted as u8);
        Ok(state)
    }

    fn load_state(&mut self, data: &[u8]) -> OmniResult<()> {
        if data.len() < 32 * 8 + 8 + 1 {
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
        Ok(())
    }

    fn backend_name(&self) -> &'static str { "rv64i-interp" }
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
        // Use mem_base=0 to avoid sign-extension issues with LUI
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        // Store 42 to address 0x100 and load it back
        // LUI produces sign-extended values, so use small addresses with ADDI only
        let insns: Vec<u32> = vec![
            0x02A00093, // ADDI x1, x0, 42
            0x10000113, // ADDI x2, x0, 256      x2 = 0x100
            0x00112023, // SW x1, 0(x2)           mem[0x100] = 42
            0x00012183, // LW x3, 0(x2)           x3 = mem[0x100]
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
            0x00208463, // BEQ x1, x2, +8        (skip next instruction)
            0x06400093, // ADDI x1, x0, 100       (should be skipped)
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 10); // x1 should still be 10, not 100
    }

    #[test]
    fn test_branch_not_taken() {
        let mut cpu = cpu_with(&[
            0x00A00093, // ADDI x1, x0, 10
            0x01400113, // ADDI x2, x0, 20
            0x00208463, // BEQ x1, x2, +8        (not taken)
            0x06400093, // ADDI x1, x0, 100       (should execute)
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 100);
    }

    #[test]
    fn test_jal() {
        let mut cpu = cpu_with(&[
            0x008000EF, // JAL x1, +8             (skip next)
            0x06400093, // ADDI x1, x0, 100       (should be skipped)
            0x00000073, // ECALL
        ]);
        let base = cpu.pc();
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), base + 4); // return address
    }

    #[test]
    fn test_uart_output() {
        let uart_addr = 0x1000_0000u64;
        let mut cpu = RiscvCpu::new("test", 4096, 0x8000_0000);
        cpu.set_uart_addr(uart_addr);
        // Build UART address in x2 via LUI + ADDI
        // LUI x2, 0x10000  =>  x2 = 0x1000_0000
        // ADDI x1, x0, 'H' (72)
        // SB x1, 0(x2)     =>  uart write 'H'
        // ECALL
        let insns: Vec<u32> = vec![
            0x10000137, // LUI x2, 0x10000        x2 = 0x1000_0000
            0x04800093, // ADDI x1, x0, 72        x1 = 'H'
            0x00110023, // SB x1, 0(x2)           store to UART
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
        // Count down from 5 to 0:
        // 0x00: ADDI x1, x0, 5       x1 = 5
        // 0x04: ADDI x1, x1, -1      x1 -= 1
        // 0x08: BNE x1, x0, -4       if x1 != 0, jump to 0x04
        // 0x0C: ECALL
        //
        // BNE x1, x0, -4:
        // offset = -4 = 0x...FFFFFFFC
        // B-type: imm[12|10:5]=1111111, rs2=00000, rs1=00001, funct3=001,
        //         imm[4:1|11]=1110_1, opcode=1100011
        // = 1_111111_00000_00001_001_1110_1_1100011
        // = 0xFE009EE3
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
            0x02A00013, // ADDI x0, x0, 42   (write to x0 — should be ignored)
            0x00000073, // ECALL
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn test_bus_backed_read_write() {
        use rhetsim_core::mem::{MemAttrs, MemAccessSize};

        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram0", 64 * 1024, 0x8000_0000));
        let mut bus = MemBus::new("sys-bus");
        bus.map(dram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
                0x8000_0000, MemAttrs::rw(), None).unwrap();
        let bus = Arc::new(bus);

        let mut cpu = RiscvCpu::with_bus("rv-core0", bus, 0x8000_0000);

        // ADDI x1, x0, 99;  SW x1, 256(x0) => store 99 at bus addr 0x80000100;
        // LW x2, 256(x0);   ECALL
        // But x0=0, so SW 0(x0) goes to addr 0 which is unmapped.
        // Use ADDI to put 0x80000000 in x3 via LUI, then SW x1, 0x100(x3)
        let insns: Vec<u32> = vec![
            0x06300093, // ADDI x1, x0, 99         x1 = 99
            0x800001B7, // LUI x3, 0x80000         x3 = 0x80000000
            0x00118213, // ADDI x4, x3, 1  -- wrong, need 0x100 offset
        ];
        // Actually simpler: use LUI+SW with offset
        // LUI x3, 0x80000 => x3 = 0x80000000 (but this is negative in 32-bit... in RV64 it's 0xFFFFFFFF80000000)
        // Hmm, LUI sign-extends the 32-bit result into 64 bits. 0x80000 << 12 = 0x80000000
        // which is negative as i32, so sign-extended to 0xFFFFFFFF80000000 in RV64.
        // Need a different approach. Use ADDI chain or put program at base 0.

        // Simpler: use mem_base=0, map DRAM at 0
        drop(cpu);
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram0", 64 * 1024, 0));
        let mut bus = MemBus::new("sys-bus");
        bus.map(dram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
                0, MemAttrs::rw(), None).unwrap();
        let bus = Arc::new(bus);

        let mut cpu = RiscvCpu::with_bus("rv-core0", bus.clone(), 0);

        let insns: Vec<u32> = vec![
            0x06300093, // ADDI x1, x0, 99         x1 = 99
            0x10000113, // ADDI x2, x0, 256        x2 = 0x100
            0x00112023, // SW x1, 0(x2)            mem[0x100] = 99
            0x00012183, // LW x3, 0(x2)            x3 = mem[0x100]
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

        // Verify data is visible in DRAM through the bus
        let val = bus.read(0x100, MemAccessSize::U32, 0).unwrap();
        assert_eq!(val, 99);
    }
}
