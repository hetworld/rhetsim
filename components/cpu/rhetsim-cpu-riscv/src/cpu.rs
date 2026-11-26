//! RiscvCpu — RV64IMAC interpreter with M/S privilege modes and Sv39 MMU

use std::sync::Arc;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::diag::trace as trace_cat;
use rhetsim_core::irq::IrqLineSource;
use rhetsim_core::mem::MemAccessSize;
use crate::csr::{self, CsrFile};
use crate::decode::{decode, decode_compressed, Instruction};
use crate::jit::{JitEngine, JitCpuState, HelperContext, FastTlbEntry, FAST_TLB_SIZE};
use crate::mmu;

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

/// RISC-V 本地中断输入线。
///
/// 这些线由平台层连接到 CLINT/PLIC，CPU 在每次检查中断前轮询并同步到
/// `mip` 对应位。未连接的线保持原有行为（测试里可以直接写 `mip`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiscvIrqLine {
    /// Machine software interrupt (CLINT MSIP)
    MSoft = 0,
    /// Machine timer interrupt (CLINT MTIP)
    MTimer = 1,
    /// Machine external interrupt (PLIC M-mode context)
    MExt = 2,
    /// Supervisor external interrupt (PLIC S-mode context)
    SExt = 3,
}

const IRQ_LINE_COUNT: usize = 4;

fn priv_name(mode: u8) -> &'static str {
    match mode {
        0 => "U",
        1 => "S",
        3 => "M",
        _ => "?",
    }
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
    priv_mode: u8,
    insn_size: u64,
    jit: Option<JitEngine>,
    tlb: mmu::Tlb,
    read_fast_tlb: Box<[FastTlbEntry; FAST_TLB_SIZE]>,
    write_fast_tlb: Box<[FastTlbEntry; FAST_TLB_SIZE]>,
    dram_host_ptr: Option<(*mut u8, u64, u64)>,
    /// Page fault raised by a JIT data-access helper, delivered on block exit.
    jit_fault: Option<(u64, u64)>,
    irq_lines: [Option<Arc<dyn IrqLineSource>>; IRQ_LINE_COUNT],
}

unsafe impl Send for RiscvCpu {}
unsafe impl Sync for RiscvCpu {}

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
            priv_mode: csr::PRV_M,
            insn_size: 4,
            jit: Some(JitEngine::new()),
            tlb: mmu::Tlb::new(),
            read_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            write_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            dram_host_ptr: None,
            jit_fault: None,
            irq_lines: std::array::from_fn(|_| None),
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
            priv_mode: csr::PRV_M,
            insn_size: 4,
            jit: Some(JitEngine::new()),
            tlb: mmu::Tlb::new(),
            read_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            write_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            dram_host_ptr: None,
            jit_fault: None,
            irq_lines: std::array::from_fn(|_| None),
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

    /// Set the hart ID reported by the `mhartid` CSR. Must be called before
    /// the CPU starts (OpenSBI and Linux use it to identify the hart).
    pub fn set_hartid(&mut self, hartid: u64) { self.csr.mhartid = hartid; }

    /// Enable or disable the JIT. Trace consumers that need instruction
    /// granularity (`exec` / `cpu`) disable it so every instruction retires
    /// through the interpreter.
    pub fn set_jit_enabled(&mut self, enabled: bool) {
        if enabled {
            if self.jit.is_none() {
                self.jit = Some(JitEngine::new());
            }
        } else {
            self.jit = None;
        }
    }

    pub fn jit_enabled(&self) -> bool { self.jit.is_some() }

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
    pub fn set_reg(&mut self, idx: usize, val: u64) { if idx != 0 && idx < 32 { self.regs[idx] = val; } }
    pub fn is_halted(&self) -> bool { self.halted }
    pub fn insn_count(&self) -> u64 { self.insn_count }
    pub fn csr(&self) -> &CsrFile { &self.csr }
    pub fn csr_mut(&mut self) -> &mut CsrFile { &mut self.csr }
    pub fn priv_mode(&self) -> u8 { self.priv_mode }
    pub fn set_priv_mode(&mut self, mode: u8) { self.priv_mode = mode; }

    pub fn set_dram_direct(&mut self, ptr: *mut u8, base: u64, size: u64) {
        self.dram_host_ptr = Some((ptr, base, size));
    }

    /// 连接一条本地中断输入线。未连接的线不参与 `mip` 同步。
    pub fn set_irq_line(&mut self, line: RiscvIrqLine, src: Arc<dyn IrqLineSource>) {
        self.irq_lines[line as usize] = Some(src);
    }

    fn sync_irq_lines(&mut self) {
        let msoft = self.irq_lines[RiscvIrqLine::MSoft as usize]
            .as_ref().map(|l| l.is_asserted());
        let mtimer = self.irq_lines[RiscvIrqLine::MTimer as usize]
            .as_ref().map(|l| l.is_asserted());
        let mext = self.irq_lines[RiscvIrqLine::MExt as usize]
            .as_ref().map(|l| l.is_asserted());
        let sext = self.irq_lines[RiscvIrqLine::SExt as usize]
            .as_ref().map(|l| l.is_asserted());

        Self::apply_mip_bit(&mut self.csr.mip, csr::MIP_MSIP, msoft);
        Self::apply_mip_bit(&mut self.csr.mip, csr::MIP_MTIP, mtimer);
        Self::apply_mip_bit(&mut self.csr.mip, csr::MIP_MEIP, mext);
        Self::apply_mip_bit(&mut self.csr.mip, csr::MIP_SEIP, sext);

        // Sstc: stimecmp is compared against the time CSR (we model time
        // as insn_count). OpenSBI uses this path when it detects SSTC,
        // instead of programming the CLINT and injecting STIP in software.
        if self.csr.sstc_active {
            let stip = self.insn_count >= self.csr.stimecmp;
            Self::apply_mip_bit(&mut self.csr.mip, csr::MIP_STIP, Some(stip));
        }
    }

    fn apply_mip_bit(mip: &mut u64, bit: u64, state: Option<bool>) {
        match state {
            Some(true) => *mip |= bit,
            Some(false) => *mip &= !bit,
            None => {}
        }
    }

    fn flush_fast_tlb(&mut self) {
        for e in self.read_fast_tlb.iter_mut() { e.tag = u64::MAX; }
        for e in self.write_fast_tlb.iter_mut() { e.tag = u64::MAX; }
    }

    // ── Physical memory access (no MMU translation) ───────────

    fn phys_read(&self, addr: u64, size: usize) -> u64 {
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

    fn phys_write(&mut self, addr: u64, size: usize, val: u64) {
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

    // ── Virtual → Physical translation ────────────────────────

    fn translate_walk(&self, va: u64, access: mmu::AccessType) -> Result<u64, mmu::PageFault> {
        mmu::translate_sv39(
            va,
            self.csr.satp,
            access,
            self.priv_mode,
            self.csr.mstatus,
            &|addr| self.phys_read(addr, 8),
        )
    }

    fn translate(&mut self, va: u64, access: mmu::AccessType) -> Result<u64, mmu::PageFault> {
        let mode = (self.csr.satp >> 60) & 0xF;
        if mode == 0 || self.priv_mode == 3 {
            return Ok(va);
        }
        if let Some(pa) = self.tlb.lookup(va, access) {
            return Ok(pa);
        }
        let pa = self.translate_walk(va, access)?;
        self.tlb.fill(va, pa, access);
        Ok(pa)
    }

    // ── Trap handling ─────────────────────────────────────────

    fn take_trap(&mut self, cause: u64, tval: u64) {
        let delegated = self.priv_mode <= csr::PRV_S && self.csr.is_delegated(cause);

        let is_interrupt = cause & (1u64 << 63) != 0;
        let kind = if is_interrupt { "interrupt" } else { "exception" };
        let to = if delegated { "S" } else { "M" };
        tracing::trace!(
            target: trace_cat::INT,
            step = self.insn_count,
            hart = self.csr.mhartid,
            kind = %kind,
            cause = format_args!("0x{:X}", cause),
            epc = format_args!("0x{:X}", self.pc),
            tval = format_args!("0x{:X}", tval),
            from = %priv_name(self.priv_mode),
            to = %to,
            "trap"
        );

        if delegated {
            let target = self.csr.trap_enter_s(self.pc, cause, tval, self.priv_mode);
            if target == 0 {
                self.halted = true;
            } else {
                self.pc = target;
                self.priv_mode = csr::PRV_S;
            }
        } else {
            let target = self.csr.trap_enter_m(self.pc, cause, tval, self.priv_mode);
            if target == 0 {
                self.halted = true;
            } else {
                self.pc = target;
                self.priv_mode = csr::PRV_M;
            }
        }
    }

    fn check_interrupt(&mut self) -> bool {
        self.sync_irq_lines();
        if let Some(cause) = self.csr.pending_interrupt(self.priv_mode) {
            self.take_trap(csr::MCAUSE_INTERRUPT | cause, 0);
            true
        } else {
            false
        }
    }

    fn trace_trap_return(&self, kind: &str, target: u64, mode: u8) {
        tracing::trace!(
            target: trace_cat::INT,
            hart = self.csr.mhartid,
            kind = %kind,
            target = format_args!("0x{:X}", target),
            from = %priv_name(self.priv_mode),
            to = %priv_name(mode),
            "trap return"
        );
    }

    // ── JIT helpers ────────────────────────────────────────────

    fn fill_fast_tlb(
        tlb: &mut [FastTlbEntry; FAST_TLB_SIZE],
        va: u64, pa: u64,
        dram: Option<(*mut u8, u64, u64)>,
    ) {
        if let Some((host_ptr, base, dram_size)) = dram {
            if pa >= base && pa < base + dram_size {
                let va_page = va >> 12;
                let idx = (va_page as usize) & (FAST_TLB_SIZE - 1);
                let addend = (host_ptr as u64)
                    .wrapping_sub(base)
                    .wrapping_add(pa & !0xFFF)
                    .wrapping_sub(va & !0xFFF);
                tlb[idx] = FastTlbEntry { tag: va_page, addend };
            }
        }
    }

    fn jit_load(cpu_ptr: *mut u8, va: u64, size: usize, fault_out: *mut u64) -> u64 {
        let cpu = unsafe { &mut *(cpu_ptr as *mut RiscvCpu) };
        let pa = match cpu.translate(va, mmu::AccessType::Load) {
            Ok(pa) => pa,
            Err(f) => {
                cpu.jit_fault = Some((f.cause, f.tval));
                unsafe { *fault_out = 1; }
                return 0;
            }
        };
        Self::fill_fast_tlb(&mut cpu.read_fast_tlb, va, pa, cpu.dram_host_ptr);
        cpu.phys_read(pa, size)
    }

    fn jit_store(cpu_ptr: *mut u8, va: u64, size: usize, val: u64, fault_out: *mut u64) -> u64 {
        let cpu = unsafe { &mut *(cpu_ptr as *mut RiscvCpu) };
        let pa = match cpu.translate(va, mmu::AccessType::Store) {
            Ok(pa) => pa,
            Err(f) => {
                cpu.jit_fault = Some((f.cause, f.tval));
                unsafe { *fault_out = 1; }
                return 1;
            }
        };
        Self::fill_fast_tlb(&mut cpu.write_fast_tlb, va, pa, cpu.dram_host_ptr);
        cpu.phys_write(pa, size, val);
        0
    }

    fn jit_step(&mut self) -> bool {
        let jit = match self.jit.as_mut() {
            Some(j) => j as *mut JitEngine,
            None => return false,
        };

        if self.dram_host_ptr.is_none() {
            if let MemBackend::Flat { memory, .. } = &self.backend {
                self.dram_host_ptr = Some((memory.as_ptr() as *mut u8, self.mem_base, memory.len() as u64));
            }
        }

        let mut state = JitCpuState {
            regs: self.regs,
            pc: self.pc,
            exit_reason: 0,
            exit_pc: 0,
            _pad: 0,
            insn_count: 0,
            raw_insn: 0,
        };

        let self_ptr = self as *mut RiscvCpu as *mut u8;
        let read_tlb_ptr = self.read_fast_tlb.as_mut_ptr();
        let write_tlb_ptr = self.write_fast_tlb.as_mut_ptr();
        let mut helper_ctx = HelperContext {
            cpu_ptr: self_ptr,
            load_fn: Self::jit_load,
            store_fn: Self::jit_store,
            read_tlb: read_tlb_ptr,
            write_tlb: write_tlb_ptr,
            fault: 0,
        };
        self.jit_fault = None;

        let priv_mode = self.priv_mode;
        let satp = self.csr.satp;

        let fetch_fn = |pc: u64| -> Result<u64, (u64, u64)> {
            let cpu = unsafe { &mut *(self_ptr as *mut RiscvCpu) };
            let pa = match cpu.translate(pc, mmu::AccessType::Fetch) {
                Ok(pa) => pa,
                Err(fault) => return Err((fault.cause, fault.tval)),
            };
            let lo = cpu.phys_read(pa, 2) as u16;
            if lo & 0x3 != 0x3 {
                Ok(lo as u64)
            } else {
                let hi = cpu.phys_read(pa + 2, 2) as u16;
                Ok(((hi as u64) << 16) | lo as u64)
            }
        };

        let jit = unsafe { &mut *jit };
        let ok = jit.execute(&mut state, &mut helper_ctx, priv_mode, satp, &fetch_fn);
        if !ok { return false; }

        self.regs = state.regs;
        self.regs[0] = 0;
        self.insn_count += state.insn_count;

        match state.exit_reason {
            3 => {
                // BLOCK_END: normal exit, pc already set
                self.pc = state.pc;
            }
            1 => {
                // ECALL
                self.pc = state.exit_pc;
                let cause = match self.priv_mode {
                    0 => 8, 1 => 9, 3 => 11, _ => 8,
                };
                self.take_trap(cause, 0);
            }
            2 => {
                // TRAP (unknown instruction)
                self.pc = state.exit_pc;
                self.take_trap(2, 0);
            }
            4 => {
                // CSR — fall back to interpreter for this instruction
                self.pc = state.exit_pc;
                self.step_one_interp();
            }
            5 => {
                // MRET
                self.pc = state.exit_pc;
                let (pc, mode) = self.csr.mret();
                self.trace_trap_return("mret", pc, mode);
                self.pc = pc;
                self.priv_mode = mode;
                self.insn_count += 1;
            }
            6 => {
                // SRET
                self.pc = state.exit_pc;
                let (pc, mode) = self.csr.sret();
                self.trace_trap_return("sret", pc, mode);
                self.pc = pc;
                self.priv_mode = mode;
                self.insn_count += 1;
            }
            7 => {
                // WFI
                self.pc = state.exit_pc;
                self.pc += 4; // WFI is always 4 bytes
                self.insn_count += 1;
            }
            8 => {
                // SFENCE
                self.pc = state.exit_pc;
                self.pc += 4;
                self.insn_count += 1;
                self.tlb.flush();
                self.flush_fast_tlb();
                if let Some(jit) = self.jit.as_mut() {
                    jit.invalidate_all();
                }
            }
            9 => {
                // AMO — fall back to interpreter
                self.pc = state.exit_pc;
                self.step_one_interp();
            }
            10 => {
                // FENCE.I
                self.pc = state.exit_pc;
                self.pc += 4;
                self.insn_count += 1;
                if let Some(jit) = self.jit.as_mut() {
                    jit.invalidate_all();
                }
            }
            11 => {
                // EBREAK
                self.pc = state.exit_pc;
                self.take_trap(3, self.pc);
            }
            12 => {
                // JIT data-access fault: re-deliver the pending page fault
                // (epc is the faulting instruction, set by the block exit).
                self.pc = state.exit_pc;
                if let Some((cause, tval)) = self.jit_fault.take() {
                    self.take_trap(cause, tval);
                }
            }
            _ => {
                self.pc = state.pc;
            }
        }

        true
    }

    fn step_one_interp(&mut self) {
        let fetch_pa = match self.translate(self.pc, mmu::AccessType::Fetch) {
            Ok(pa) => pa,
            Err(fault) => {
                self.take_trap(fault.cause, fault.tval);
                return;
            }
        };

        let half = self.phys_read(fetch_pa, 2) as u16;
        if half == 0 {
            self.halted = true;
            return;
        }

        let (raw_insn, insn) = if half & 0x3 != 0x3 {
            self.insn_size = 2;
            (half as u32, decode_compressed(half))
        } else {
            self.insn_size = 4;
            let raw = (self.phys_read(fetch_pa + 2, 2) as u32) << 16 | half as u32;
            (raw, decode(raw))
        };

        tracing::trace!(
            target: trace_cat::EXEC,
            step = self.insn_count,
            hart = self.csr.mhartid,
            pc = format_args!("0x{:08X}", self.pc),
            priv = %priv_name(self.priv_mode),
            insn = format_args!("0x{:08X}", raw_insn),
            decoded = ?insn,
            "exec"
        );

        self.execute(insn);
        self.regs[0] = 0;
        self.insn_count += 1;

        if tracing::enabled!(target: trace_cat::CPU, tracing::Level::TRACE) {
            use std::fmt::Write as _;
            let mut regs = String::with_capacity(32 * 18);
            for (idx, value) in self.regs.iter().enumerate().skip(1) {
                if idx > 1 {
                    regs.push(' ');
                }
                let _ = write!(regs, "x{idx}=0x{value:016X}");
            }
            tracing::trace!(
                target: trace_cat::CPU,
                step = self.insn_count,
                hart = self.csr.mhartid,
                pc = format_args!("0x{:08X}", self.pc),
                priv = %priv_name(self.priv_mode),
                regs = %regs,
                "cpu"
            );
        }
    }

    // ── Execute ───────────────────────────────────────────────

    fn set_rd(&mut self, rd: u8, val: u64) {
        if rd != 0 {
            self.regs[rd as usize] = val;
        }
    }

    fn execute(&mut self, insn: Instruction) {
        match insn {
            Instruction::Lui { rd, imm } => {
                self.set_rd(rd, imm as u64);
                self.pc += self.insn_size;
            }
            Instruction::Auipc { rd, imm } => {
                self.set_rd(rd, self.pc.wrapping_add(imm as u64));
                self.pc += self.insn_size;
            }
            Instruction::Jal { rd, offset } => {
                self.set_rd(rd, self.pc + self.insn_size);
                self.pc = self.pc.wrapping_add(offset as u64);
            }
            Instruction::Jalr { rd, rs1, offset } => {
                let target = (self.regs[rs1 as usize].wrapping_add(offset as u64)) & !1;
                self.set_rd(rd, self.pc + self.insn_size);
                self.pc = target;
            }
            Instruction::Branch { funct3, rs1, rs2, offset } => {
                let a = self.regs[rs1 as usize];
                let b = self.regs[rs2 as usize];
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
                    self.pc = self.pc.wrapping_add(offset as u64);
                } else {
                    self.pc += self.insn_size;
                }
            }
            Instruction::Load { funct3, rd, rs1, offset } => {
                let va = self.regs[rs1 as usize].wrapping_add(offset as u64);
                let pa = match self.translate(va, mmu::AccessType::Load) {
                    Ok(pa) => pa,
                    Err(fault) => { self.take_trap(fault.cause, fault.tval); return; }
                };
                let val = match funct3 {
                    0 => self.phys_read(pa, 1) as i8 as i64 as u64,
                    1 => self.phys_read(pa, 2) as i16 as i64 as u64,
                    2 => self.phys_read(pa, 4) as i32 as i64 as u64,
                    3 => self.phys_read(pa, 8),
                    4 => self.phys_read(pa, 1),
                    5 => self.phys_read(pa, 2),
                    6 => self.phys_read(pa, 4),
                    _ => 0,
                };
                self.set_rd(rd, val);
                self.pc += self.insn_size;
            }
            Instruction::Store { funct3, rs1, rs2, offset } => {
                let va = self.regs[rs1 as usize].wrapping_add(offset as u64);
                let pa = match self.translate(va, mmu::AccessType::Store) {
                    Ok(pa) => pa,
                    Err(fault) => { self.take_trap(fault.cause, fault.tval); return; }
                };
                let val = self.regs[rs2 as usize];
                match funct3 {
                    0 => self.phys_write(pa, 1, val),
                    1 => self.phys_write(pa, 2, val),
                    2 => self.phys_write(pa, 4, val),
                    3 => self.phys_write(pa, 8, val),
                    _ => {}
                }
                self.pc += self.insn_size;
            }
            Instruction::OpImm { funct3, rd, rs1, imm, funct7 } => {
                let src = self.regs[rs1 as usize];
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
                self.set_rd(rd, val);
                self.pc += self.insn_size;
            }
            Instruction::Op { funct3, funct7, rd, rs1, rs2 } => {
                let a = self.regs[rs1 as usize];
                let b = self.regs[rs2 as usize];
                let val = if funct7 == 0x01 {
                    self.exec_m(funct3, a, b)
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
                self.set_rd(rd, val);
                self.pc += self.insn_size;
            }
            Instruction::OpImm32 { funct3, rd, rs1, imm, funct7 } => {
                let src = self.regs[rs1 as usize] as u32;
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
                self.set_rd(rd, val as i32 as i64 as u64);
                self.pc += self.insn_size;
            }
            Instruction::Op32 { funct3, funct7, rd, rs1, rs2 } => {
                let a = self.regs[rs1 as usize] as u32;
                let b = self.regs[rs2 as usize] as u32;
                let val = if funct7 == 0x01 {
                    self.exec_m32(funct3, a, b)
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
                self.set_rd(rd, val as i32 as i64 as u64);
                self.pc += self.insn_size;
            }
            Instruction::Csr { funct3, rd, rs1, csr } => {
                let csr_priv = ((csr >> 8) & 0x3) as u8;
                if csr_priv > self.priv_mode {
                    self.take_trap(2, 0);
                    return;
                }
                let old = match self.csr.read(csr, self.insn_count) {
                    Some(v) => v,
                    None => {
                        // Unsupported CSR: illegal instruction.
                        self.take_trap(2, 0);
                        return;
                    }
                };
                let src = self.regs[rs1 as usize];
                match funct3 {
                    1 => {
                        self.set_rd(rd, old);
                        self.csr.write(csr, src);
                    }
                    2 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 { self.csr.write(csr, old | src); }
                    }
                    3 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 { self.csr.write(csr, old & !src); }
                    }
                    5 => {
                        self.set_rd(rd, old);
                        self.csr.write(csr, rs1 as u64);
                    }
                    6 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 { self.csr.write(csr, old | rs1 as u64); }
                    }
                    7 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 { self.csr.write(csr, old & !(rs1 as u64)); }
                    }
                    _ => {}
                }
                if csr == 0x180 { self.tlb.flush(); self.flush_fast_tlb(); }
                self.pc += self.insn_size;
            }
            Instruction::Mret => {
                let (pc, mode) = self.csr.mret();
                self.trace_trap_return("mret", pc, mode);
                self.pc = pc;
                self.priv_mode = mode;
            }
            Instruction::Sret => {
                let (pc, mode) = self.csr.sret();
                self.trace_trap_return("sret", pc, mode);
                self.pc = pc;
                self.priv_mode = mode;
            }
            Instruction::Wfi => {
                self.pc += self.insn_size;
            }
            Instruction::SfenceVma { .. } => {
                self.tlb.flush();
                self.flush_fast_tlb();
                self.pc += self.insn_size;
            }
            Instruction::Amo { funct5, aq: _, rl: _, funct3, rd, rs1, rs2 } => {
                let va = self.regs[rs1 as usize];
                let access = if funct5 == 0x02 { mmu::AccessType::Load } else { mmu::AccessType::Store };
                let pa = match self.translate(va, access) {
                    Ok(pa) => pa,
                    Err(fault) => { self.take_trap(fault.cause, fault.tval); return; }
                };
                self.exec_amo(funct5, funct3, rd, rs2, pa);
                self.pc += self.insn_size;
            }
            Instruction::Ecall => {
                let cause = match self.priv_mode {
                    0 => 8,
                    1 => 9,
                    3 => 11,
                    _ => 8,
                };
                self.take_trap(cause, 0);
            }
            Instruction::Ebreak => {
                self.take_trap(3, self.pc);
            }
            Instruction::Fence | Instruction::FenceI => {
                self.pc += self.insn_size;
            }
            Instruction::Unknown(_) => {
                self.take_trap(2, 0);
            }
        }
    }

    fn exec_m(&self, funct3: u8, a: u64, b: u64) -> u64 {
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
                let sa = a as i64;
                let sb = b as i64;
                if sb == 0 { u64::MAX }
                else if sa == i64::MIN && sb == -1 { sa as u64 }
                else { sa.wrapping_div(sb) as u64 }
            }
            5 => { if b == 0 { u64::MAX } else { a / b } }
            6 => {
                let sa = a as i64;
                let sb = b as i64;
                if sb == 0 { a }
                else if sa == i64::MIN && sb == -1 { 0 }
                else { sa.wrapping_rem(sb) as u64 }
            }
            7 => { if b == 0 { a } else { a % b } }
            _ => 0,
        }
    }

    fn exec_m32(&self, funct3: u8, a: u32, b: u32) -> u32 {
        match funct3 {
            0 => a.wrapping_mul(b),
            4 => {
                let sa = a as i32;
                let sb = b as i32;
                if sb == 0 { u32::MAX }
                else if sa == i32::MIN && sb == -1 { sa as u32 }
                else { sa.wrapping_div(sb) as u32 }
            }
            5 => { if b == 0 { u32::MAX } else { a / b } }
            6 => {
                let sa = a as i32;
                let sb = b as i32;
                if sb == 0 { a }
                else if sa == i32::MIN && sb == -1 { 0 }
                else { sa.wrapping_rem(sb) as u32 }
            }
            7 => { if b == 0 { a } else { a % b } }
            _ => 0,
        }
    }

    fn exec_amo(&mut self, funct5: u8, funct3: u8, rd: u8, rs2: u8, pa: u64) {
        let src2 = self.regs[rs2 as usize];
        let is_word = funct3 == 2;

        match funct5 {
            0x02 => { // LR
                let val = if is_word {
                    self.phys_read(pa, 4) as i32 as i64 as u64
                } else {
                    self.phys_read(pa, 8)
                };
                self.set_rd(rd, val);
                self.reservation = Some(pa);
            }
            0x03 => { // SC
                if self.reservation == Some(pa) {
                    if is_word { self.phys_write(pa, 4, src2); }
                    else { self.phys_write(pa, 8, src2); }
                    self.set_rd(rd, 0);
                } else {
                    self.set_rd(rd, 1);
                }
                self.reservation = None;
            }
            _ => {
                let (old, new_val) = if is_word {
                    let old = self.phys_read(pa, 4) as i32 as i64 as u64;
                    let s2 = src2 as i32;
                    let o = old as i32;
                    let nv = match funct5 {
                        0x01 => src2 as u32,
                        0x00 => o.wrapping_add(s2) as u32,
                        0x04 => (old as u32) ^ (src2 as u32),
                        0x0C => (old as u32) & (src2 as u32),
                        0x08 => (old as u32) | (src2 as u32),
                        0x10 => o.min(s2) as u32,
                        0x14 => o.max(s2) as u32,
                        0x18 => (old as u32).min(src2 as u32),
                        0x1C => (old as u32).max(src2 as u32),
                        _ => old as u32,
                    };
                    (old, nv as i32 as i64 as u64)
                } else {
                    let old = self.phys_read(pa, 8);
                    let nv = match funct5 {
                        0x01 => src2,
                        0x00 => (old as i64).wrapping_add(src2 as i64) as u64,
                        0x04 => old ^ src2,
                        0x0C => old & src2,
                        0x08 => old | src2,
                        0x10 => (old as i64).min(src2 as i64) as u64,
                        0x14 => (old as i64).max(src2 as i64) as u64,
                        0x18 => old.min(src2),
                        0x1C => old.max(src2),
                        _ => old,
                    };
                    (old, nv)
                };
                if is_word { self.phys_write(pa, 4, new_val); }
                else { self.phys_write(pa, 8, new_val); }
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
        self.priv_mode = csr::PRV_M;
        self.insn_size = 4;
        if let MemBackend::Flat { uart_output, .. } = &mut self.backend {
            uart_output.clear();
        }
        self.tlb.flush();
        self.flush_fast_tlb();
        self.jit_fault = None;
        if let Some(jit) = self.jit.as_mut() {
            jit.invalidate_all();
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
            self.check_interrupt();
            if self.halted { break; }

            let before = self.insn_count;
            let enter_pc = self.pc;
            let enter_priv = self.priv_mode;
            let used_jit = self.jit_step();
            if !used_jit {
                self.step_one_interp();
            }
            let ran = self.insn_count - before;
            if used_jit {
                tracing::trace!(
                    target: trace_cat::JIT,
                    hart = self.csr.mhartid,
                    pc = format_args!("0x{:08X}", enter_pc),
                    priv = %priv_name(enter_priv),
                    insns = ran,
                    next_pc = format_args!("0x{:08X}", self.pc),
                    "block"
                );
            }
            executed += ran.max(1);
        }
        Ok(executed)
    }

    fn reset(&mut self) -> OmniResult<()> {
        Unit::reset(self)
    }

    fn save_state(&self) -> OmniResult<Vec<u8>> {
        let csr_data = self.csr.save();
        let mut state = Vec::with_capacity(32 * 8 + 8 + 1 + 1 + csr_data.len());
        for r in &self.regs {
            state.extend_from_slice(&r.to_le_bytes());
        }
        state.extend_from_slice(&self.pc.to_le_bytes());
        state.push(self.halted as u8);
        state.push(self.priv_mode);
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
        if data.len() > 265 {
            self.priv_mode = data[265];
        }
        let csr_offset = 266;
        if data.len() >= csr_offset + CsrFile::state_size() {
            self.csr.load(&data[csr_offset..]);
        } else if data.len() >= min_size + 1 + 8 * 8 {
            // Legacy format
            self.csr.load(&data[min_size + 1..]);
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str { "rv64imac-interp" }
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
            0x00A00093, 0x00A00113, 0x00208463, 0x06400093,
            0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 10);
    }

    #[test]
    fn test_branch_not_taken() {
        let mut cpu = cpu_with(&[
            0x00A00093, 0x01400113, 0x00208463, 0x06400093,
            0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 100);
    }

    #[test]
    fn test_jal() {
        let mut cpu = cpu_with(&[
            0x008000EF, 0x06400093, 0x00000073,
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
            0x10000137, 0x04800093, 0x00110023, 0x00000073,
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
            0x02A00093, 0x00000073,
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
            0x00500093, 0xFFF08093, 0xFE009EE3, 0x00000073,
        ]);
        cpu.exec(100).unwrap();
        assert_eq!(cpu.reg(1), 0);
        assert!(cpu.is_halted());
    }

    #[test]
    fn test_x0_always_zero() {
        let mut cpu = cpu_with(&[
            0x02A00013, 0x00000073,
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
            0x06300093, 0x10000113, 0x00112023, 0x00012183,
            0x00000073,
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

    #[test]
    fn test_csrrw() {
        let csrrw = (0x340 << 20) | (2 << 15) | (1 << 12) | (1 << 7) | 0x73;
        let mut cpu = cpu_with(&[
            0x06400113, csrrw, 0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 0);
        assert_eq!(cpu.csr().mscratch, 100);
    }

    #[test]
    fn test_csrrs_mie() {
        let csrrs = (0x304 << 20) | (1 << 15) | (2 << 12) | (2 << 7) | 0x73;
        let mut cpu = cpu_with(&[
            0x08000093, csrrs, 0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.csr().mie, 128);
    }

    #[test]
    fn test_mul() {
        let mul = (0x01 << 25) | (2 << 20) | (1 << 15) | (0 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x00700093, 0x00600113, mul, 0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 42);
    }

    #[test]
    fn test_div() {
        let div = (0x01 << 25) | (2 << 20) | (1 << 15) | (4 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x02A00093, 0x00700113, div, 0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 6);
    }

    #[test]
    fn test_div_by_zero() {
        let div = (0x01 << 25) | (2 << 20) | (1 << 15) | (4 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x02A00093, 0x00000113, div, 0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), u64::MAX);
    }

    #[test]
    fn test_rem() {
        let rem = (0x01 << 25) | (2 << 20) | (1 << 15) | (6 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[
            0x02B00093, 0x00700113, rem, 0x00000073,
        ]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 1);
    }

    #[test]
    fn test_lr_sc_w() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        let sw     = (0 << 25) | (1 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let lr_w   = (0x02 << 27) | (0 << 20) | (2 << 15) | (2 << 12) | (3 << 7) | 0x2F;
        let sc_w   = (0x03 << 27) | (4 << 20) | (2 << 15) | (2 << 12) | (5 << 7) | 0x2F;
        let lw_x6  = (0 << 20) | (2 << 15) | (2 << 12) | (6 << 7) | 0x03;
        let insns: Vec<u32> = vec![
            0x02A00093, 0x10000113, sw, lr_w, 0x06300213, sc_w, lw_x6,
            0x00000073,
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(20).unwrap();
        assert_eq!(cpu.reg(3), 42);
        assert_eq!(cpu.reg(5), 0);
        assert_eq!(cpu.reg(6), 99);
    }

    #[test]
    fn test_amoadd_w() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        let sw       = (0 << 25) | (1 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let amoadd_w = (0x00 << 27) | (4 << 20) | (2 << 15) | (2 << 12) | (3 << 7) | 0x2F;
        let lw_x5    = (0 << 20) | (2 << 15) | (2 << 12) | (5 << 7) | 0x03;
        let insns: Vec<u32> = vec![
            0x00A00093, 0x10000113, sw, 0x00500213, amoadd_w, lw_x5,
            0x00000073,
        ];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(20).unwrap();
        assert_eq!(cpu.reg(3), 10);
        assert_eq!(cpu.reg(5), 15);
    }

    #[test]
    fn test_trap_delivery_and_mret() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        let csrrw_mtvec    = (0x305 << 20) | (1 << 15) | (1 << 12) | (0 << 7) | 0x73;
        let csrrs_mie      = (0x304 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x73;
        let csrrs_mstatus  = (0x300 << 20) | (3 << 15) | (2 << 12) | (0 << 7) | 0x73;
        let main_code: Vec<u32> = vec![
            0x10000093, csrrw_mtvec, 0x08000113, csrrs_mie,
            0x00800193, csrrs_mstatus,
            0x02A00593, 0x00000073,
        ];
        for (i, insn) in main_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        let mret = 0x30200073u32;
        let csrrc_mie = (0x304u32 << 20) | (9 << 15) | (3 << 12) | (0 << 7) | 0x73;
        let handler: Vec<u32> = vec![
            0x00100513, 0x08000493, csrrc_mie, mret,
        ];
        for (i, insn) in handler.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x100 + i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(6).unwrap();
        assert!(!cpu.is_halted());
        cpu.csr_mut().mip |= csr::MIP_MTIP;
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(10), 1);
        assert_eq!(cpu.reg(11), 42);
    }

    // ── Privilege mode tests ──────────────────────────────────

    #[test]
    fn test_m_to_s_mode_transition() {
        // M-mode code: set mepc, set MPP=S, MRET → enter S-mode
        // S-mode code: set x10 = 1, ECALL → back to M-mode
        // M-mode handler: set x11 = 1, halt
        let mut cpu = RiscvCpu::new("test", 0x10000, 0);

        // mepc = 0x200 (S-mode entry)
        let csrrw_mepc = (0x341u32 << 20) | (1 << 15) | (1 << 12) | (0 << 7) | 0x73;
        // mstatus: set MPP to S-mode (01 << 11) and MIE
        let csrrw_mstatus = (0x300u32 << 20) | (2 << 15) | (1 << 12) | (0 << 7) | 0x73;
        // mtvec = 0x100 (M-mode trap handler)
        let csrrw_mtvec = (0x305u32 << 20) | (3 << 15) | (1 << 12) | (0 << 7) | 0x73;
        let mret = 0x30200073u32;

        let m_code: Vec<u32> = vec![
            0x20000093,         // ADDI x1, x0, 0x200 (S-mode entry)
            csrrw_mepc,         // CSRRW x0, mepc, x1
            0x00800113,         // ADDI x2, x0, 0x008 (MIE bit)
            // MPP = S-mode (1 << 11 = 0x800)
            0x00001137,         // LUI x2, 0x1
            0x80010113,         // ADDI x2, x2, -0x800 → x2 = 0x800
            csrrw_mstatus,      // CSRRW x0, mstatus, x2
            0x10000193,         // ADDI x3, x0, 0x100 (handler)
            csrrw_mtvec,        // CSRRW x0, mtvec, x3
            mret,               // MRET → S-mode at 0x200
        ];
        for (i, insn) in m_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }

        // S-mode code at 0x200
        let s_code: Vec<u32> = vec![
            0x00100513,         // ADDI x10, x0, 1
            0x00000073,         // ECALL → trap to M-mode (cause 9)
        ];
        for (i, insn) in s_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x200 + i * 4) as u64);
        }

        // M-mode trap handler at 0x100
        let handler: Vec<u32> = vec![
            0x00100593,         // ADDI x11, x0, 1
        ];
        // After this instruction, next is 0x104 which is 0 → halt
        for (i, insn) in handler.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x100 + i * 4) as u64);
        }

        cpu.set_pc(0);
        cpu.exec(50).unwrap();

        assert_eq!(cpu.reg(10), 1);  // S-mode executed
        assert_eq!(cpu.reg(11), 1);  // M-mode handler executed
        assert_eq!(cpu.csr().mcause, 9);  // ecall from S-mode
    }
}
