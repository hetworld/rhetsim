//! RiscvCpu — RV64IMAC interpreter with M/S privilege modes and Sv39 MMU

use crate::csr::{self, CsrFile};
use crate::decode::{decode, decode_compressed, Instruction};
use crate::jit::{FastTlbEntry, HelperContext, JitCpuState, JitEngine, FAST_TLB_SIZE};
use crate::mmu;
use crate::pmp;
use crate::profile::RiscvCpuType;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::diag::trace as trace_cat;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::irq::IrqLineSource;
use rhetsim_core::mem::MemAccessSize;
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::sync::Arc;

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
    cpu_type: RiscvCpuType,
    regs: [u64; 32],
    fregs: [u64; 32],
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
    // Diagnostic counters, printed when RHETSIM_JIT_STATS is set.
    slow_loads: u64,
    slow_stores: u64,
    sfence_count: u64,
    satp_writes: u64,
    irq_lines: [Option<Arc<dyn IrqLineSource>>; IRQ_LINE_COUNT],
}

unsafe impl Send for RiscvCpu {}
unsafe impl Sync for RiscvCpu {}

impl RiscvCpu {
    pub fn new(id: &str, mem_size: usize, mem_base: u64) -> Self {
        Self::new_with_type(id, mem_size, mem_base, RiscvCpuType::default())
    }

    pub fn new_with_type(id: &str, mem_size: usize, mem_base: u64, cpu_type: RiscvCpuType) -> Self {
        Self {
            id: UnitId::new(id),
            cpu_type,
            regs: [0; 32],
            fregs: [0; 32],
            pc: mem_base,
            halted: false,
            mem_base,
            insn_count: 0,
            backend: MemBackend::Flat {
                memory: vec![0u8; mem_size],
                uart_output: Vec::new(),
                uart_addr: 0,
            },
            csr: CsrFile::new_with_type(0, cpu_type),
            reservation: None,
            priv_mode: csr::PRV_M,
            insn_size: 4,
            jit: Some(JitEngine::new_for(cpu_type)),
            tlb: mmu::Tlb::new(),
            read_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            write_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            dram_host_ptr: None,
            jit_fault: None,
            slow_loads: 0,
            slow_stores: 0,
            sfence_count: 0,
            satp_writes: 0,
            irq_lines: std::array::from_fn(|_| None),
        }
    }

    pub fn with_bus(id: &str, bus: Arc<MemBus>, mem_base: u64) -> Self {
        Self::with_bus_type(id, bus, mem_base, RiscvCpuType::default())
    }

    pub fn with_bus_type(
        id: &str,
        bus: Arc<MemBus>,
        mem_base: u64,
        cpu_type: RiscvCpuType,
    ) -> Self {
        Self {
            id: UnitId::new(id),
            cpu_type,
            regs: [0; 32],
            fregs: [0; 32],
            pc: mem_base,
            halted: false,
            mem_base,
            insn_count: 0,
            backend: MemBackend::Bus { bus },
            csr: CsrFile::new_with_type(0, cpu_type),
            reservation: None,
            priv_mode: csr::PRV_M,
            insn_size: 4,
            jit: Some(JitEngine::new_for(cpu_type)),
            tlb: mmu::Tlb::new(),
            read_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            write_fast_tlb: Box::new([FastTlbEntry::invalid(); FAST_TLB_SIZE]),
            dram_host_ptr: None,
            jit_fault: None,
            slow_loads: 0,
            slow_stores: 0,
            sfence_count: 0,
            satp_writes: 0,
            irq_lines: std::array::from_fn(|_| None),
        }
    }

    pub fn cpu_type(&self) -> RiscvCpuType {
        self.cpu_type
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

    pub fn set_pc(&mut self, pc: u64) {
        self.pc = pc;
    }

    /// Set the hart ID reported by the `mhartid` CSR. Must be called before
    /// the CPU starts (OpenSBI and Linux use it to identify the hart).
    pub fn set_hartid(&mut self, hartid: u64) {
        self.csr.mhartid = hartid;
    }

    /// Enable or disable the JIT. Trace consumers that need instruction
    /// granularity (`exec` / `cpu`) disable it so every instruction retires
    /// through the interpreter.
    pub fn set_jit_enabled(&mut self, enabled: bool) {
        if enabled {
            if self.jit.is_none() {
                self.jit = Some(JitEngine::new_for(self.cpu_type));
            }
        } else {
            self.jit = None;
        }
    }

    pub fn jit_enabled(&self) -> bool {
        self.jit.is_some()
    }

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

    pub fn pc(&self) -> u64 {
        self.pc
    }
    pub fn reg(&self, idx: usize) -> u64 {
        self.regs[idx]
    }

    /// Floating-point register bit pattern (NaN-boxed for single precision).
    pub fn freg(&self, idx: usize) -> u64 {
        self.fregs[idx]
    }

    /// JIT counters `(compiled blocks, executions, invalidations, fallbacks)`
    /// when the JIT is enabled.
    pub fn jit_stats(&self) -> Option<(u64, u64, u64, u64)> {
        self.jit.as_ref().map(|jit| {
            let (compiles, executions, invalidations) = jit.stats();
            (compiles, executions, invalidations, jit.fallback_count())
        })
    }
    pub fn set_reg(&mut self, idx: usize, val: u64) {
        if idx != 0 && idx < 32 {
            self.regs[idx] = val;
        }
    }
    pub fn is_halted(&self) -> bool {
        self.halted
    }
    pub fn insn_count(&self) -> u64 {
        self.insn_count
    }
    pub fn csr(&self) -> &CsrFile {
        &self.csr
    }
    pub fn csr_mut(&mut self) -> &mut CsrFile {
        &mut self.csr
    }
    pub fn priv_mode(&self) -> u8 {
        self.priv_mode
    }
    pub fn set_priv_mode(&mut self, mode: u8) {
        self.enter_priv(mode);
    }

    /// Switch privilege level; translation caches are mode-sensitive.
    fn enter_priv(&mut self, mode: u8) {
        if mode != self.priv_mode {
            self.priv_mode = mode;
            self.tlb.flush();
            self.flush_fast_tlb();
        }
    }

    pub fn set_dram_direct(&mut self, ptr: *mut u8, base: u64, size: u64) {
        self.dram_host_ptr = Some((ptr, base, size));
    }

    /// 连接一条本地中断输入线。未连接的线不参与 `mip` 同步。
    pub fn set_irq_line(&mut self, line: RiscvIrqLine, src: Arc<dyn IrqLineSource>) {
        self.irq_lines[line as usize] = Some(src);
    }

    fn sync_irq_lines(&mut self) {
        let msoft = self.irq_lines[RiscvIrqLine::MSoft as usize]
            .as_ref()
            .map(|l| l.is_asserted());
        let mtimer = self.irq_lines[RiscvIrqLine::MTimer as usize]
            .as_ref()
            .map(|l| l.is_asserted());
        let mext = self.irq_lines[RiscvIrqLine::MExt as usize]
            .as_ref()
            .map(|l| l.is_asserted());
        let sext = self.irq_lines[RiscvIrqLine::SExt as usize]
            .as_ref()
            .map(|l| l.is_asserted());

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
        for e in self.read_fast_tlb.iter_mut() {
            e.tag = u64::MAX;
        }
        for e in self.write_fast_tlb.iter_mut() {
            e.tag = u64::MAX;
        }
    }

    // ── Physical memory access (no MMU translation) ───────────

    fn phys_read(&self, addr: u64, size: usize) -> u64 {
        match &self.backend {
            MemBackend::Flat {
                memory, uart_addr, ..
            } => {
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
            MemBackend::Flat {
                memory,
                uart_output,
                uart_addr,
            } => {
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

    /// Privilege level used for a data access. `mstatus.MPRV` makes M-mode
    /// perform loads and stores as the mode recorded in `MPP`.
    fn effective_priv(&self, access: mmu::AccessType) -> u8 {
        if !matches!(access, mmu::AccessType::Fetch)
            && self.priv_mode == csr::PRV_M
            && self.csr.mstatus & csr::MSTATUS_MPRV != 0
        {
            ((self.csr.mstatus & csr::MSTATUS_MPP_MASK) >> 11) as u8
        } else {
            self.priv_mode
        }
    }

    fn translate_walk(
        &self,
        va: u64,
        access: mmu::AccessType,
        priv_mode: u8,
    ) -> Result<u64, mmu::PageFault> {
        mmu::translate_sv39(
            va,
            self.csr.satp,
            access,
            priv_mode,
            self.csr.mstatus,
            &|addr| self.phys_read(addr, 8),
        )
    }

    /// Translate a virtual address and enforce PMP on the resulting
    /// physical range.
    fn translate(
        &mut self,
        va: u64,
        access: mmu::AccessType,
        size: u64,
    ) -> Result<u64, mmu::PageFault> {
        let priv_mode = self.effective_priv(access);
        let mode = (self.csr.satp >> 60) & 0xF;
        let pa = if mode == 0 || priv_mode == csr::PRV_M {
            va
        } else if let Some(pa) = self.tlb.lookup(va, access) {
            pa
        } else {
            let pa = self.translate_walk(va, access, priv_mode)?;
            self.tlb.fill(va, pa, access);
            pa
        };
        if !pmp::check(&self.csr, priv_mode, pa, size, access) {
            return Err(mmu::PageFault::access_fault(access, va));
        }
        Ok(pa)
    }

    // ── Trap handling ─────────────────────────────────────────

    fn take_trap(&mut self, cause: u64, tval: u64) {
        let delegated = self.priv_mode <= csr::PRV_S && self.csr.is_delegated(cause);

        let is_interrupt = cause & (1u64 << 63) != 0;
        let kind = if is_interrupt {
            "interrupt"
        } else {
            "exception"
        };
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
                self.enter_priv(csr::PRV_S);
            }
        } else {
            let target = self.csr.trap_enter_m(self.pc, cause, tval, self.priv_mode);
            if target == 0 {
                self.halted = true;
            } else {
                self.pc = target;
                self.enter_priv(csr::PRV_M);
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
        va: u64,
        pa: u64,
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
                tlb[idx] = FastTlbEntry {
                    tag: va_page,
                    addend,
                };
            }
        }
    }

    fn jit_load(cpu_ptr: *mut u8, va: u64, size: usize, fault_out: *mut u64) -> u64 {
        let cpu = unsafe { &mut *(cpu_ptr as *mut RiscvCpu) };
        cpu.slow_loads += 1;
        let pa = match cpu.translate(va, mmu::AccessType::Load, size as u64) {
            Ok(pa) => pa,
            Err(f) => {
                cpu.jit_fault = Some((f.cause, f.tval));
                unsafe {
                    *fault_out = 1;
                }
                return 0;
            }
        };
        Self::fill_fast_tlb(&mut cpu.read_fast_tlb, va, pa, cpu.dram_host_ptr);
        cpu.phys_read(pa, size)
    }

    fn jit_store(cpu_ptr: *mut u8, va: u64, size: usize, val: u64, fault_out: *mut u64) -> u64 {
        let cpu = unsafe { &mut *(cpu_ptr as *mut RiscvCpu) };
        cpu.slow_stores += 1;
        let pa = match cpu.translate(va, mmu::AccessType::Store, size as u64) {
            Ok(pa) => pa,
            Err(f) => {
                cpu.jit_fault = Some((f.cause, f.tval));
                unsafe {
                    *fault_out = 1;
                }
                return 1;
            }
        };
        Self::fill_fast_tlb(&mut cpu.write_fast_tlb, va, pa, cpu.dram_host_ptr);
        cpu.phys_write(pa, size, val);
        0
    }

    fn jit_cbo_zero(cpu_ptr: *mut u8, base: u64, fault_out: *mut u64) -> u64 {
        let cpu = unsafe { &mut *(cpu_ptr as *mut RiscvCpu) };
        let base = base & !0x3F;
        for offset in (0..64).step_by(8) {
            let va = base + offset;
            let pa = match cpu.translate(va, mmu::AccessType::Store, 8) {
                Ok(pa) => pa,
                Err(f) => {
                    cpu.jit_fault = Some((f.cause, f.tval));
                    unsafe {
                        *fault_out = 1;
                    }
                    return 1;
                }
            };
            cpu.phys_write(pa, 8, 0);
        }
        0
    }

    fn jit_step(&mut self) -> bool {
        let jit = match self.jit.as_mut() {
            Some(j) => j as *mut JitEngine,
            None => return false,
        };

        if self.dram_host_ptr.is_none() {
            if let MemBackend::Flat { memory, .. } = &self.backend {
                self.dram_host_ptr = Some((
                    memory.as_ptr() as *mut u8,
                    self.mem_base,
                    memory.len() as u64,
                ));
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
            fregs: self.fregs.as_mut_ptr(),
            fcsr: self.csr.fcsr as u64,
            fp_dirty: 0,
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
            cbo_zero_fn: Self::jit_cbo_zero,
        };
        self.jit_fault = None;

        let priv_mode = self.priv_mode;
        let satp = self.csr.satp;

        let fetch_fn = |pc: u64| -> Result<u64, (u64, u64)> {
            let cpu = unsafe { &mut *(self_ptr as *mut RiscvCpu) };
            let pa = match cpu.translate(pc, mmu::AccessType::Fetch, 2) {
                Ok(pa) => pa,
                Err(fault) => return Err((fault.cause, fault.tval)),
            };
            let lo = cpu.phys_read(pa, 2) as u16;
            if lo & 0x3 != 0x3 {
                Ok(lo as u64)
            } else {
                if !pmp::check(
                    &cpu.csr,
                    cpu.effective_priv(mmu::AccessType::Fetch),
                    pa,
                    4,
                    mmu::AccessType::Fetch,
                ) {
                    return Err((1, pc));
                }
                let hi = cpu.phys_read(pa + 2, 2) as u16;
                Ok(((hi as u64) << 16) | lo as u64)
            }
        };

        let jit = unsafe { &mut *jit };
        let ok = jit.execute(&mut state, &mut helper_ctx, priv_mode, satp, &fetch_fn);
        if !ok {
            return false;
        }

        // FP registers were updated in place through `state.fregs`; the
        // control/status register travels by value.
        self.csr.fcsr = state.fcsr as u32;
        if state.fp_dirty != 0 {
            self.csr.mstatus |= csr::MSTATUS_FS_DIRTY;
        }
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
                    0 => 8,
                    1 => 9,
                    3 => 11,
                    _ => 8,
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
                self.enter_priv(mode);
                self.insn_count += 1;
            }
            6 => {
                // SRET
                self.pc = state.exit_pc;
                let (pc, mode) = self.csr.sret();
                self.trace_trap_return("sret", pc, mode);
                self.pc = pc;
                self.enter_priv(mode);
                self.insn_count += 1;
            }
            7 => {
                // WFI
                self.pc = state.exit_pc;
                self.pc += 4; // WFI is always 4 bytes
                self.insn_count += 1;
            }
            8 => {
                // SFENCE — flush translation caches only. Translated blocks do
                // not embed translations (they use the runtime TLB), so the
                // native code stays valid; instruction-memory changes are
                // handled by FENCE.I instead.
                self.pc = state.exit_pc;
                self.pc += 4;
                self.insn_count += 1;
                self.sfence_count += 1;
                self.tlb.flush();
                self.flush_fast_tlb();
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
        let fetch_pa = match self.translate(self.pc, mmu::AccessType::Fetch, 2) {
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
            if !self.cpu_type.has_compressed() {
                self.take_trap(2, 0);
                return;
            }
            self.insn_size = 2;
            (half as u32, decode_compressed(half))
        } else {
            self.insn_size = 4;
            let mode = self.effective_priv(mmu::AccessType::Fetch);
            if !pmp::check(&self.csr, mode, fetch_pa, 4, mmu::AccessType::Fetch) {
                self.take_trap(1, self.pc);
                return;
            }
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

    // ── Floating-point helpers ────────────────────────────────

    fn fp32(&self, r: u8) -> f32 {
        f32::from_bits(self.fregs[r as usize] as u32)
    }

    fn fp64(&self, r: u8) -> f64 {
        f64::from_bits(self.fregs[r as usize])
    }

    fn set_fp32(&mut self, r: u8, value: f32) {
        self.fregs[r as usize] = 0xFFFF_FFFF_0000_0000 | value.to_bits() as u64;
    }

    fn set_fp32_bits(&mut self, r: u8, bits: u32) {
        self.fregs[r as usize] = 0xFFFF_FFFF_0000_0000 | bits as u64;
    }

    fn set_fp64(&mut self, r: u8, value: f64) {
        self.fregs[r as usize] = value.to_bits();
    }

    fn mark_fp_dirty(&mut self) {
        self.csr.mstatus |= csr::MSTATUS_FS_DIRTY;
    }

    fn set_fflag(&mut self, flag: u32) {
        self.csr.fcsr |= flag;
        self.mark_fp_dirty();
    }

    fn execute_fp_load(&mut self, width: u8, rd: u8, rs1: u8, offset: i64) {
        let va = self.regs[rs1 as usize].wrapping_add(offset as u64);
        let pa = match self.translate(va, mmu::AccessType::Load, width as u64) {
            Ok(pa) => pa,
            Err(fault) => {
                self.take_trap(fault.cause, fault.tval);
                return;
            }
        };
        if width == 4 {
            self.set_fp32_bits(rd, self.phys_read(pa, 4) as u32);
        } else {
            self.fregs[rd as usize] = self.phys_read(pa, 8);
        }
        self.mark_fp_dirty();
        self.pc += self.insn_size;
    }

    fn execute_fp_store(&mut self, width: u8, rs1: u8, rs2: u8, offset: i64) {
        let va = self.regs[rs1 as usize].wrapping_add(offset as u64);
        let pa = match self.translate(va, mmu::AccessType::Store, width as u64) {
            Ok(pa) => pa,
            Err(fault) => {
                self.take_trap(fault.cause, fault.tval);
                return;
            }
        };
        let value = if width == 4 {
            self.fregs[rs2 as usize] & 0xFFFF_FFFF
        } else {
            self.fregs[rs2 as usize]
        };
        self.phys_write(pa, width as usize, value);
        self.mark_fp_dirty();
        self.pc += self.insn_size;
    }

    fn fp_to_int(&mut self, rd: u8, value: f64, kind: u8) {
        if value.is_nan() {
            self.set_fflag(1 << 4); // invalid
        }
        let out = match kind {
            0 => value as i32 as i64 as u64,
            1 => value as u32 as u64,
            2 => value as i64 as u64,
            _ => value as u64,
        };
        self.set_rd(rd, out);
    }

    fn int_to_fp(&mut self, rs1: u8, kind: u8) -> f64 {
        let value = self.regs[rs1 as usize];
        match kind {
            0 => value as i32 as f64,
            1 => value as u32 as f64,
            2 => value as i64 as f64,
            _ => value as f64,
        }
    }

    /// RISC-V FMIN/FMAX semantics: a single NaN operand is ignored, two NaN
    /// operands produce the canonical NaN, and two zeros keep the sign the
    /// instruction requires.
    fn fp_minmax(a: f64, b: f64, take_max: bool) -> f64 {
        if a.is_nan() && b.is_nan() {
            return f64::from_bits(0x7FF8_0000_0000_0000);
        }
        if a.is_nan() {
            return b;
        }
        if b.is_nan() {
            return a;
        }
        if a == 0.0 && b == 0.0 {
            return if take_max { 0.0 } else { -0.0 };
        }
        if take_max {
            a.max(b)
        } else {
            a.min(b)
        }
    }

    fn fp_class(bits: u64, double: bool) -> u64 {
        let (sign, exp, frac, qnan) = if double {
            (
                bits >> 63,
                (bits >> 52) & 0x7FF,
                bits & 0x000F_FFFF_FFFF_FFFF,
                1u64 << 51,
            )
        } else {
            (
                (bits >> 31) & 1,
                (bits >> 23) & 0xFF,
                bits & 0x007F_FFFF,
                1u64 << 22,
            )
        };
        let negative = sign != 0;
        if exp == if double { 0x7FF } else { 0xFF } {
            return if frac == 0 {
                if negative {
                    1 << 0
                } else {
                    1 << 7
                }
            } else if frac & qnan != 0 {
                1 << 9
            } else {
                1 << 8
            };
        }
        if exp == 0 {
            if frac == 0 {
                if negative {
                    1 << 3
                } else {
                    1 << 4
                }
            } else if negative {
                1 << 2
            } else {
                1 << 5
            }
        } else if negative {
            1 << 1
        } else {
            1 << 6
        }
    }

    fn execute_fp_op(&mut self, funct7: u8, funct3: u8, rd: u8, rs1: u8, rs2: u8) {
        let is_double = funct7 & 1 != 0;
        match funct7 {
            0x00 => {
                let v = self.fp32(rs1) + self.fp32(rs2);
                self.set_fp32(rd, v);
            }
            0x01 => {
                let v = self.fp64(rs1) + self.fp64(rs2);
                self.set_fp64(rd, v);
            }
            0x04 => {
                let v = self.fp32(rs1) - self.fp32(rs2);
                self.set_fp32(rd, v);
            }
            0x05 => {
                let v = self.fp64(rs1) - self.fp64(rs2);
                self.set_fp64(rd, v);
            }
            0x08 => {
                let v = self.fp32(rs1) * self.fp32(rs2);
                self.set_fp32(rd, v);
            }
            0x09 => {
                let v = self.fp64(rs1) * self.fp64(rs2);
                self.set_fp64(rd, v);
            }
            0x0C => {
                let (a, b) = (self.fp32(rs1), self.fp32(rs2));
                if b == 0.0 {
                    self.set_fflag(if a == 0.0 { 1 << 4 } else { 1 << 3 });
                }
                self.set_fp32(rd, a / b);
            }
            0x0D => {
                let (a, b) = (self.fp64(rs1), self.fp64(rs2));
                if b == 0.0 {
                    self.set_fflag(if a == 0.0 { 1 << 4 } else { 1 << 3 });
                }
                self.set_fp64(rd, a / b);
            }
            0x2C => {
                let a = self.fp32(rs1);
                if a < 0.0 {
                    self.set_fflag(1 << 4);
                }
                self.set_fp32(rd, a.sqrt());
            }
            0x2D => {
                let a = self.fp64(rs1);
                if a < 0.0 {
                    self.set_fflag(1 << 4);
                }
                self.set_fp64(rd, a.sqrt());
            }
            0x10 | 0x11 => {
                let (a, b, sign_mask) = if is_double {
                    (
                        self.fregs[rs1 as usize],
                        self.fregs[rs2 as usize],
                        1u64 << 63,
                    )
                } else {
                    (
                        self.fregs[rs1 as usize] & 0xFFFF_FFFF,
                        self.fregs[rs2 as usize] & 0xFFFF_FFFF,
                        1u64 << 31,
                    )
                };
                let sign = match funct3 {
                    0 => b & sign_mask,
                    1 => (!b) & sign_mask,
                    _ => (a ^ b) & sign_mask,
                };
                let out = (a & !sign_mask) | sign;
                if is_double {
                    self.fregs[rd as usize] = out;
                } else {
                    self.set_fp32_bits(rd, out as u32);
                }
            }
            0x14 | 0x15 => {
                let take_max = funct3 != 0;
                if is_double {
                    let (a, b) = (self.fp64(rs1), self.fp64(rs2));
                    self.set_fp64(rd, Self::fp_minmax(a, b, take_max));
                } else {
                    let (a, b) = (self.fp32(rs1), self.fp32(rs2));
                    self.set_fp32(rd, Self::fp_minmax(a as f64, b as f64, take_max) as f32);
                }
            }
            0x20 => {
                let v = self.fp64(rs1) as f32;
                self.set_fp32(rd, v);
            }
            0x21 => {
                let v = self.fp32(rs1) as f64;
                self.set_fp64(rd, v);
            }
            0x50 | 0x51 => {
                let result = if is_double {
                    let (a, b) = (self.fp64(rs1), self.fp64(rs2));
                    match funct3 {
                        2 => a == b,
                        1 => a < b,
                        _ => a <= b,
                    }
                } else {
                    let (a, b) = (self.fp32(rs1), self.fp32(rs2));
                    match funct3 {
                        2 => a == b,
                        1 => a < b,
                        _ => a <= b,
                    }
                };
                self.set_rd(rd, result as u64);
            }
            0x60 | 0x61 => {
                let value = if is_double {
                    self.fp64(rs1)
                } else {
                    self.fp32(rs1) as f64
                };
                self.fp_to_int(rd, value, rs2);
            }
            0x68 | 0x69 => {
                let value = self.int_to_fp(rs1, rs2);
                if is_double {
                    self.set_fp64(rd, value);
                } else {
                    self.set_fp32(rd, value as f32);
                }
            }
            0x70 => {
                if funct3 == 1 {
                    let bits = self.fregs[rs1 as usize] & 0xFFFF_FFFF;
                    self.set_rd(rd, Self::fp_class(bits, false));
                } else {
                    let val = self.fregs[rs1 as usize] as u32 as i32 as i64 as u64;
                    self.set_rd(rd, val);
                }
            }
            0x71 => {
                if funct3 == 1 {
                    self.set_rd(rd, Self::fp_class(self.fregs[rs1 as usize], true));
                } else {
                    self.set_rd(rd, self.fregs[rs1 as usize]);
                }
            }
            0x78 => self.set_fp32_bits(rd, self.regs[rs1 as usize] as u32),
            0x79 => self.fregs[rd as usize] = self.regs[rs1 as usize],
            _ => {
                self.take_trap(2, 0);
                return;
            }
        }
        self.mark_fp_dirty();
        self.pc += self.insn_size;
    }

    fn execute_fp_fma(&mut self, opcode: u8, funct7: u8, rd: u8, rs1: u8, rs2: u8, rs3: u8) {
        let is_double = funct7 & 1 != 0;
        if is_double {
            let (a, b, c) = (self.fp64(rs1), self.fp64(rs2), self.fp64(rs3));
            let value = match opcode {
                0x43 => a.mul_add(b, c),
                0x47 => a.mul_add(b, -c),
                0x4B => (-a).mul_add(b, c),
                _ => (-a).mul_add(b, -c),
            };
            self.set_fp64(rd, value);
        } else {
            let (a, b, c) = (self.fp32(rs1), self.fp32(rs2), self.fp32(rs3));
            let value = match opcode {
                0x43 => a.mul_add(b, c),
                0x47 => a.mul_add(b, -c),
                0x4B => (-a).mul_add(b, c),
                _ => (-a).mul_add(b, -c),
            };
            self.set_fp32(rd, value);
        }
        self.mark_fp_dirty();
        self.pc += self.insn_size;
    }

    fn execute_cbo(&mut self, op: u8, rs1: u8) {
        match op {
            0..=2 => {}
            4 => {
                let base = self.regs[rs1 as usize] & !0x3F;
                for offset in (0..64).step_by(8) {
                    let va = base + offset;
                    let pa = match self.translate(va, mmu::AccessType::Store, 8) {
                        Ok(pa) => pa,
                        Err(fault) => {
                            self.take_trap(fault.cause, fault.tval);
                            return;
                        }
                    };
                    self.phys_write(pa, 8, 0);
                }
            }
            _ => {
                self.take_trap(2, 0);
                return;
            }
        }
        self.pc += self.insn_size;
    }

    // ── T-Head XThead vendor instructions ─────────────────────

    /// Execute one T-Head XThead instruction (opcode `0x0B`).
    fn execute_thead(&mut self, raw: u32) {
        let f3 = ((raw >> 12) & 0x7) as u8;
        let rd = ((raw >> 7) & 0x1F) as u8;
        let rs1 = ((raw >> 15) & 0x1F) as u8;
        let rs2 = ((raw >> 20) & 0x1F) as u8;
        let hi6 = (raw >> 26) & 0x3F;

        match f3 {
            0 => {
                // XTheadCmo cache maintenance is a no-op for this model.
                if rd == 0 && (raw >> 25) & 0x7F <= 1 {
                    self.pc += self.insn_size;
                } else {
                    self.take_trap(2, 0);
                }
            }
            1 => self.execute_thead_alu(raw, rd, rs1, rs2, hi6),
            2 | 3 => self.execute_thead_extract(raw, rd, rs1, f3 == 2),
            4 | 5 => self.execute_thead_mem(raw, rd, rs1, rs2, f3 == 4),
            6 | 7 => self.execute_thead_fmem(raw, rd, rs1, rs2, f3 == 6),
            _ => self.take_trap(2, 0),
        }
    }

    /// XTheadBa/Bb/Bs/CondMov/Fmv/Mac register operations.
    fn execute_thead_alu(&mut self, raw: u32, rd: u8, rs1: u8, rs2: u8, hi6: u32) {
        let dst = rd as usize;
        let src1 = self.regs[rs1 as usize];
        let src2 = self.regs[rs2 as usize];
        let acc = self.regs[dst];

        match hi6 {
            // th.srri (rotate right) and th.tst (bit test).
            0b000100 => {
                let shamt = (raw >> 20) & 0x3F;
                self.set_rd(rd, src1.rotate_right(shamt));
            }
            0b100010 => {
                let shamt = (raw >> 20) & 0x3F;
                self.set_rd(rd, (src1 >> shamt) & 1);
            }
            _ => {
                let funct7 = ((raw >> 25) & 0x7F) as u8;
                // These encodings fix the rs2 field to zero.
                if rs2 != 0 && matches!(funct7, 0x40..=0x43 | 0x48 | 0x50 | 0x60) {
                    self.take_trap(2, 0);
                    return;
                }
                match funct7 {
                    0x00 => self.set_rd(rd, src1.wrapping_add(src2)),
                    0x01..=0x03 => {
                        // th.addsl{1,2,3}: shift is applied to rs2.
                        self.set_rd(rd, src1.wrapping_add(src2 << funct7));
                    }
                    0x40 => {
                        // th.tstnbz: inverted orc.b.
                        let mut out = 0u64;
                        for byte in 0..8 {
                            if (src1 >> (byte * 8)) & 0xFF == 0 {
                                out |= 0xFF << (byte * 8);
                            }
                        }
                        self.set_rd(rd, out);
                    }
                    0x41 => self.set_rd(rd, src1.swap_bytes()),
                    0x42 => self.set_rd(rd, (!src1).leading_zeros() as u64),
                    0x43 => self.set_rd(rd, src1.leading_zeros() as u64),
                    0x48 => {
                        let v = (src1 as u32).swap_bytes();
                        self.set_rd(rd, v as i32 as i64 as u64);
                    }
                    0x50 => {
                        // th.fmv.hw.x: f[rd][63:32] = rs1[31:0].
                        self.fregs[dst] =
                            (self.fregs[dst] & 0xFFFF_FFFF) | ((src1 & 0xFFFF_FFFF) << 32);
                        self.mark_fp_dirty();
                    }
                    0x60 => {
                        // th.fmv.x.hw: rd = f[rs1][63:32].
                        self.set_rd(rd, (self.fregs[rs1 as usize] >> 32) & 0xFFFF_FFFF);
                    }
                    0x10 => self.set_rd(rd, acc.wrapping_add(src1.wrapping_mul(src2))),
                    0x11 => self.set_rd(rd, acc.wrapping_sub(src1.wrapping_mul(src2))),
                    0x12 => {
                        let v = (acc as u32).wrapping_add(src1.wrapping_mul(src2) as u32);
                        self.set_rd(rd, v as i32 as i64 as u64);
                    }
                    0x13 => {
                        let v = (acc as u32).wrapping_sub(src1.wrapping_mul(src2) as u32);
                        self.set_rd(rd, v as i32 as i64 as u64);
                    }
                    0x14 => {
                        let p = (src1 as i16 as i32).wrapping_mul(src2 as i16 as i32);
                        let v = (acc as u32).wrapping_add(p as u32);
                        self.set_rd(rd, v as i32 as i64 as u64);
                    }
                    0x15 => {
                        let p = (src1 as i16 as i32).wrapping_mul(src2 as i16 as i32);
                        let v = (acc as u32).wrapping_sub(p as u32);
                        self.set_rd(rd, v as i32 as i64 as u64);
                    }
                    0x20 => {
                        // th.mveqz
                        if src2 == 0 {
                            self.set_rd(rd, src1);
                        }
                    }
                    0x21 => {
                        // th.mvnez
                        if src2 != 0 {
                            self.set_rd(rd, src1);
                        }
                    }
                    _ => {
                        self.take_trap(2, 0);
                        return;
                    }
                }
            }
        }
        self.pc += self.insn_size;
    }

    /// th.ext / th.extu bitfield extraction.
    fn execute_thead_extract(&mut self, raw: u32, rd: u8, rs1: u8, signed: bool) {
        let msb = (raw >> 26) & 0x3F;
        let lsb = (raw >> 20) & 0x3F;
        if lsb <= msb {
            let width = msb - lsb + 1;
            let mask = if width == 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            let mut val = (self.regs[rs1 as usize] >> lsb) & mask;
            if signed && val & (1u64 << (width - 1)) != 0 {
                val |= !mask;
            }
            self.set_rd(rd, val);
        }
        self.pc += self.insn_size;
    }

    fn thead_load_val(&mut self, va: u64, size: u64, signed: bool) -> Option<u64> {
        let pa = match self.translate(va, mmu::AccessType::Load, size) {
            Ok(pa) => pa,
            Err(fault) => {
                self.take_trap(fault.cause, fault.tval);
                return None;
            }
        };
        Some(match (size, signed) {
            (1, true) => self.phys_read(pa, 1) as i8 as i64 as u64,
            (1, false) => self.phys_read(pa, 1) & 0xFF,
            (2, true) => self.phys_read(pa, 2) as i16 as i64 as u64,
            (2, false) => self.phys_read(pa, 2) & 0xFFFF,
            (4, true) => self.phys_read(pa, 4) as i32 as i64 as u64,
            (4, false) => self.phys_read(pa, 4) & 0xFFFF_FFFF,
            _ => self.phys_read(pa, 8),
        })
    }

    fn thead_store_val(&mut self, va: u64, size: u64, val: u64) -> bool {
        let pa = match self.translate(va, mmu::AccessType::Store, size) {
            Ok(pa) => pa,
            Err(fault) => {
                self.take_trap(fault.cause, fault.tval);
                return false;
            }
        };
        self.phys_write(pa, size as usize, val);
        true
    }

    /// XTheadMemIdx / XTheadMemPair memory operations.
    fn execute_thead_mem(&mut self, raw: u32, rd: u8, rs1: u8, rs2: u8, load: bool) {
        let hi5 = (raw >> 27) & 0x1F;
        let imm2 = (raw >> 25) & 0x3;
        let imm5 = ((((raw >> 20) & 0x1F) << 27) as i32 >> 27) as i64;
        let inc_imm = (imm5 as u64) << imm2;
        let base = self.regs[rs1 as usize];

        // Increment forms (th.ldia/ldib, th.sdia/sdib, ...).
        let inc = match hi5 {
            0x0F | 0x0D => Some((8, true)),
            0x0B | 0x09 => Some((4, true)),
            0x1B | 0x19 => Some((4, false)),
            0x07 | 0x05 => Some((2, true)),
            0x17 | 0x15 => Some((2, false)),
            0x03 | 0x01 => Some((1, true)),
            0x13 | 0x11 => Some((1, false)),
            _ => None,
        };
        if let Some((size, signed)) = inc {
            if load {
                if rs1 == rd {
                    self.take_trap(2, 0);
                    return;
                }
                let preinc = hi5 & 0x2 == 0;
                let va = base.wrapping_add(if preinc { inc_imm } else { 0 });
                let Some(val) = self.thead_load_val(va, size, signed) else {
                    return;
                };
                self.regs[rs1 as usize] = base.wrapping_add(inc_imm);
                self.set_rd(rd, val);
            } else {
                if !matches!(hi5, 0x0F | 0x0D | 0x0B | 0x09 | 0x07 | 0x05 | 0x03 | 0x01) {
                    self.take_trap(2, 0);
                    return;
                }
                let preinc = hi5 & 0x2 == 0;
                let va = base.wrapping_add(if preinc { inc_imm } else { 0 });
                let val = self.regs[rd as usize];
                if !self.thead_store_val(va, size, val) {
                    return;
                }
                self.regs[rs1 as usize] = base.wrapping_add(inc_imm);
            }
            self.pc += self.insn_size;
            return;
        }

        // Indexed forms (th.lrd/..., th.lurd/..., th.srd/..., th.surd/...).
        let idx = match hi5 {
            0x0C => Some((8, true, false)),
            0x08 => Some((4, true, false)),
            0x18 => Some((4, false, false)),
            0x04 => Some((2, true, false)),
            0x14 => Some((2, false, false)),
            0x00 => Some((1, true, false)),
            0x10 => Some((1, false, false)),
            0x0E => Some((8, true, true)),
            0x0A => Some((4, true, true)),
            0x1A => Some((4, false, true)),
            0x06 => Some((2, true, true)),
            0x16 => Some((2, false, true)),
            0x02 => Some((1, true, true)),
            0x12 => Some((1, false, true)),
            _ => None,
        };
        if let Some((size, signed, zext_offs)) = idx {
            if !load && !matches!(hi5, 0x0C | 0x08 | 0x04 | 0x00 | 0x0E | 0x0A | 0x06 | 0x02) {
                self.take_trap(2, 0);
                return;
            }
            let offset = if zext_offs {
                self.regs[rs2 as usize] & 0xFFFF_FFFF
            } else {
                self.regs[rs2 as usize]
            };
            let va = base.wrapping_add(offset << imm2);
            if load {
                let Some(val) = self.thead_load_val(va, size, signed) else {
                    return;
                };
                self.set_rd(rd, val);
            } else {
                let val = self.regs[rd as usize];
                if !self.thead_store_val(va, size, val) {
                    return;
                }
            }
            self.pc += self.insn_size;
            return;
        }

        // Pair forms (th.ldd/lwd/lwud/sdd/swd). rd2 lives in bits 24:20.
        let pair_ok = if load {
            matches!(hi5, 0x1F | 0x1C | 0x1E)
        } else {
            matches!(hi5, 0x1F | 0x1C)
        };
        if !pair_ok {
            self.take_trap(2, 0);
            return;
        }
        let (size, signed, shamt) = match hi5 {
            0x1F => (8u64, false, 4u32),
            0x1E => (4, false, 3),
            _ => (4, true, 3),
        };
        if load && (rs1 == rd || rs1 == rs2 || rd == rs2) {
            self.take_trap(2, 0);
            return;
        }
        let imm = (imm2 as u64) << shamt;
        let va1 = base.wrapping_add(imm);
        let va2 = va1.wrapping_add(size);
        if load {
            let Some(v1) = self.thead_load_val(va1, size, signed) else {
                return;
            };
            let Some(v2) = self.thead_load_val(va2, size, signed) else {
                return;
            };
            self.set_rd(rd, v1);
            self.set_rd(rs2, v2);
        } else {
            let v1 = self.regs[rd as usize];
            let v2 = self.regs[rs2 as usize];
            if !self.thead_store_val(va1, size, v1) {
                return;
            }
            if !self.thead_store_val(va2, size, v2) {
                return;
            }
        }
        self.pc += self.insn_size;
    }

    /// XTheadFMemIdx floating-point indexed loads and stores.
    fn execute_thead_fmem(&mut self, raw: u32, rd: u8, rs1: u8, rs2: u8, load: bool) {
        let hi5 = (raw >> 27) & 0x1F;
        let imm2 = (raw >> 25) & 0x3;
        let (size, zext_offs) = match hi5 {
            0x0C => (8u64, false),
            0x08 => (4, false),
            0x0E => (8, true),
            0x0A => (4, true),
            _ => {
                self.take_trap(2, 0);
                return;
            }
        };
        let offset = if zext_offs {
            self.regs[rs2 as usize] & 0xFFFF_FFFF
        } else {
            self.regs[rs2 as usize]
        };
        let va = self.regs[rs1 as usize].wrapping_add(offset << imm2);
        if load {
            let pa = match self.translate(va, mmu::AccessType::Load, size) {
                Ok(pa) => pa,
                Err(fault) => {
                    self.take_trap(fault.cause, fault.tval);
                    return;
                }
            };
            if size == 4 {
                self.set_fp32_bits(rd, self.phys_read(pa, 4) as u32);
            } else {
                self.fregs[rd as usize] = self.phys_read(pa, 8);
            }
            self.mark_fp_dirty();
        } else {
            let pa = match self.translate(va, mmu::AccessType::Store, size) {
                Ok(pa) => pa,
                Err(fault) => {
                    self.take_trap(fault.cause, fault.tval);
                    return;
                }
            };
            let val = if size == 4 {
                self.fregs[rd as usize] & 0xFFFF_FFFF
            } else {
                self.fregs[rd as usize]
            };
            self.phys_write(pa, size as usize, val);
        }
        self.pc += self.insn_size;
    }

    fn execute_bitmanip(&mut self, opcode: u8, funct7: u8, funct3: u8, rd: u8, rs1: u8, rs2: u8) {
        let a = self.regs[rs1 as usize];
        let b = self.regs[rs2 as usize];
        let value = match (opcode, funct7, funct3) {
            (0x33, 0x10, 2) => a.wrapping_shl(1).wrapping_add(b),
            (0x33, 0x10, 4) => a.wrapping_shl(2).wrapping_add(b),
            (0x33, 0x10, 6) => a.wrapping_shl(3).wrapping_add(b),
            (0x33, 0x20, 4) => !(a ^ b),
            (0x33, 0x20, 6) => a | !b,
            (0x33, 0x20, 7) => a & !b,
            (0x33, 0x05, 1) => Self::clmul(a, b).0,
            (0x33, 0x05, 2) => {
                let (lo, hi) = Self::clmul(a, b);
                (lo >> 63) | (hi << 1)
            }
            (0x33, 0x05, 3) => Self::clmul(a, b).1,
            (0x33, 0x05, 4) => (a as i64).min(b as i64) as u64,
            (0x33, 0x05, 5) => a.min(b),
            (0x33, 0x05, 6) => (a as i64).max(b as i64) as u64,
            (0x33, 0x05, 7) => a.max(b),
            (0x33, 0x14, 1) => a | (1u64 << (b & 63)),
            (0x33, 0x24, 1) => a & !(1u64 << (b & 63)),
            (0x33, 0x24, 5) => (a >> (b & 63)) & 1,
            (0x33, 0x34, 1) => a ^ (1u64 << (b & 63)),
            (0x33, 0x30, 1) => a.rotate_left((b & 63) as u32),
            (0x33, 0x30, 5) => a.rotate_right((b & 63) as u32),
            (0x3B, 0x04, 0) => (a as u32 as u64).wrapping_add(b),
            (0x3B, 0x04, 4) => a & 0xFFFF,
            (0x3B, 0x10, 2) => (a as u32 as u64).wrapping_shl(1).wrapping_add(b),
            (0x3B, 0x10, 4) => (a as u32 as u64).wrapping_shl(2).wrapping_add(b),
            (0x3B, 0x10, 6) => (a as u32 as u64).wrapping_shl(3).wrapping_add(b),
            (0x3B, 0x30, 1) => (a as u32).rotate_left((b & 31) as u32) as i32 as i64 as u64,
            (0x3B, 0x30, 5) => (a as u32).rotate_right((b & 31) as u32) as i32 as i64 as u64,
            _ => {
                self.take_trap(2, 0);
                return;
            }
        };
        self.set_rd(rd, value);
        self.pc += self.insn_size;
    }

    fn clmul(a: u64, b: u64) -> (u64, u64) {
        let mut lo = 0u64;
        let mut hi = 0u64;
        for i in 0..64 {
            if (b >> i) & 1 != 0 {
                lo ^= a << i;
                if i != 0 {
                    hi ^= a >> (64 - i);
                }
            }
        }
        (lo, hi)
    }

    fn execute_bitmanip_imm(&mut self, opcode: u8, funct3: u8, rd: u8, rs1: u8, funct12: u16) {
        let a = self.regs[rs1 as usize];
        let value = if opcode == 0x13 && funct3 == 5 {
            match funct12 {
                0x6B8 => a.swap_bytes(),
                0x287 => {
                    let mut out = 0u64;
                    for (i, byte) in a.to_le_bytes().iter().enumerate() {
                        if *byte != 0 {
                            out |= 0xFFu64 << (i * 8);
                        }
                    }
                    out
                }
                f if (0x600..0x640).contains(&f) => a.rotate_right((f & 0x3F) as u32),
                f if (0x480..0x4C0).contains(&f) => (a >> (f & 0x3F)) & 1,
                _ => {
                    self.take_trap(2, 0);
                    return;
                }
            }
        } else if opcode == 0x1B && funct3 == 1 {
            match funct12 {
                0x600 => (a as u32).leading_zeros() as u64,
                0x601 => (a as u32).trailing_zeros() as u64,
                0x602 => (a as u32).count_ones() as u64,
                f if f >> 5 == 0x04 || f >> 5 == 0x05 => (a as u32 as u64) << (f & 0x3F),
                _ => {
                    self.take_trap(2, 0);
                    return;
                }
            }
        } else if opcode == 0x1B && funct3 == 5 {
            if !(0x600..0x640).contains(&funct12) {
                self.take_trap(2, 0);
                return;
            }
            (a as u32).rotate_right((funct12 & 0x1F) as u32) as i32 as i64 as u64
        } else if opcode == 0x13 && funct3 == 1 {
            match funct12 {
                0x600 => a.leading_zeros() as u64,
                0x601 => a.trailing_zeros() as u64,
                0x602 => a.count_ones() as u64,
                0x604 => a as i8 as i64 as u64,
                0x605 => a as i16 as i64 as u64,
                f if (0x280..0x2C0).contains(&f) => a | (1u64 << (f & 0x3F)),
                f if (0x480..0x4C0).contains(&f) => a & !(1u64 << (f & 0x3F)),
                f if (0x680..0x6C0).contains(&f) => a ^ (1u64 << (f & 0x3F)),
                _ => {
                    self.take_trap(2, 0);
                    return;
                }
            }
        } else {
            self.take_trap(2, 0);
            return;
        };
        self.set_rd(rd, value);
        self.pc += self.insn_size;
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
            Instruction::Branch {
                funct3,
                rs1,
                rs2,
                offset,
            } => {
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
            Instruction::Load {
                funct3,
                rd,
                rs1,
                offset,
            } => {
                let va = self.regs[rs1 as usize].wrapping_add(offset as u64);
                let size = match funct3 {
                    0 | 4 => 1,
                    1 | 5 => 2,
                    2 | 6 => 4,
                    _ => 8,
                };
                let pa = match self.translate(va, mmu::AccessType::Load, size) {
                    Ok(pa) => pa,
                    Err(fault) => {
                        self.take_trap(fault.cause, fault.tval);
                        return;
                    }
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
            Instruction::Store {
                funct3,
                rs1,
                rs2,
                offset,
            } => {
                let va = self.regs[rs1 as usize].wrapping_add(offset as u64);
                let size = match funct3 {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    _ => 8,
                };
                let pa = match self.translate(va, mmu::AccessType::Store, size) {
                    Ok(pa) => pa,
                    Err(fault) => {
                        self.take_trap(fault.cause, fault.tval);
                        return;
                    }
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
            Instruction::FpLoad {
                width,
                rd,
                rs1,
                offset,
            } => self.execute_fp_load(width, rd, rs1, offset),
            Instruction::FpStore {
                width,
                rs1,
                rs2,
                offset,
            } => self.execute_fp_store(width, rs1, rs2, offset),
            Instruction::FpOp {
                funct7,
                funct3,
                rd,
                rs1,
                rs2,
            } => self.execute_fp_op(funct7, funct3, rd, rs1, rs2),
            Instruction::FpFma {
                opcode,
                funct7,
                rd,
                rs1,
                rs2,
                rs3,
            } => self.execute_fp_fma(opcode, funct7, rd, rs1, rs2, rs3),
            Instruction::Bitmanip {
                opcode,
                funct7,
                funct3,
                rd,
                rs1,
                rs2,
            } => self.execute_bitmanip(opcode, funct7, funct3, rd, rs1, rs2),
            Instruction::BitmanipImm {
                opcode,
                funct3,
                rd,
                rs1,
                funct12,
            } => self.execute_bitmanip_imm(opcode, funct3, rd, rs1, funct12),
            Instruction::Cbo { op, rs1 } => self.execute_cbo(op, rs1),
            Instruction::TheadSync => {
                self.pc += self.insn_size;
            }
            Instruction::Thead { raw } => self.execute_thead(raw),
            Instruction::OpImm {
                funct3,
                rd,
                rs1,
                imm,
                funct7,
            } => {
                let src = self.regs[rs1 as usize];
                let shamt = (imm as u64) & 0x3F;
                let val = match funct3 {
                    0 => src.wrapping_add(imm as u64),
                    1 => src << shamt,
                    2 => ((src as i64) < imm) as u64,
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
            Instruction::Op {
                funct3,
                funct7,
                rd,
                rs1,
                rs2,
            } => {
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
            Instruction::OpImm32 {
                funct3,
                rd,
                rs1,
                imm,
                funct7,
            } => {
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
            Instruction::Op32 {
                funct3,
                funct7,
                rd,
                rs1,
                rs2,
            } => {
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
            Instruction::Csr {
                funct3,
                rd,
                rs1,
                csr,
            } => {
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
                        if rs1 != 0 {
                            self.csr.write(csr, old | src);
                        }
                    }
                    3 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old & !src);
                        }
                    }
                    5 => {
                        self.set_rd(rd, old);
                        self.csr.write(csr, rs1 as u64);
                    }
                    6 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old | rs1 as u64);
                        }
                    }
                    7 => {
                        self.set_rd(rd, old);
                        if rs1 != 0 {
                            self.csr.write(csr, old & !(rs1 as u64));
                        }
                    }
                    _ => {}
                }
                match csr {
                    csr::SATP => {
                        self.satp_writes += 1;
                        self.tlb.flush();
                        self.flush_fast_tlb();
                    }
                    csr::MSTATUS => {
                        // MPRV/MPP change the translation and PMP context of
                        // subsequent data accesses.
                        if (old ^ self.csr.mstatus) & (csr::MSTATUS_MPRV | csr::MSTATUS_MPP_MASK)
                            != 0
                        {
                            self.tlb.flush();
                            self.flush_fast_tlb();
                        }
                    }
                    csr::PMPCFG0 | csr::PMPCFG2 | csr::MSECCFG | 0x3B0..=0x3BF => {
                        self.tlb.flush();
                        self.flush_fast_tlb();
                        if let Some(jit) = self.jit.as_mut() {
                            jit.invalidate_all();
                        }
                    }
                    _ => {}
                }
                self.pc += self.insn_size;
            }
            Instruction::Mret => {
                let (pc, mode) = self.csr.mret();
                self.trace_trap_return("mret", pc, mode);
                self.pc = pc;
                self.enter_priv(mode);
            }
            Instruction::Sret => {
                let (pc, mode) = self.csr.sret();
                self.trace_trap_return("sret", pc, mode);
                self.pc = pc;
                self.enter_priv(mode);
            }
            Instruction::Wfi => {
                self.pc += self.insn_size;
            }
            Instruction::SfenceVma { .. } => {
                self.tlb.flush();
                self.flush_fast_tlb();
                self.pc += self.insn_size;
            }
            Instruction::Amo {
                funct5,
                aq: _,
                rl: _,
                funct3,
                rd,
                rs1,
                rs2,
            } => {
                let va = self.regs[rs1 as usize];
                let access = if funct5 == 0x02 {
                    mmu::AccessType::Load
                } else {
                    mmu::AccessType::Store
                };
                let size = if funct3 == 2 { 4 } else { 8 };
                let pa = match self.translate(va, access, size) {
                    Ok(pa) => pa,
                    Err(fault) => {
                        self.take_trap(fault.cause, fault.tval);
                        return;
                    }
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
                if sb == 0 {
                    u64::MAX
                } else if sa == i64::MIN && sb == -1 {
                    sa as u64
                } else {
                    sa.wrapping_div(sb) as u64
                }
            }
            5 => a.checked_div(b).unwrap_or(u64::MAX),
            6 => {
                let sa = a as i64;
                let sb = b as i64;
                if sb == 0 {
                    a
                } else if sa == i64::MIN && sb == -1 {
                    0
                } else {
                    sa.wrapping_rem(sb) as u64
                }
            }
            7 => {
                if b == 0 {
                    a
                } else {
                    a % b
                }
            }
            _ => 0,
        }
    }

    fn exec_m32(&self, funct3: u8, a: u32, b: u32) -> u32 {
        match funct3 {
            0 => a.wrapping_mul(b),
            4 => {
                let sa = a as i32;
                let sb = b as i32;
                if sb == 0 {
                    u32::MAX
                } else if sa == i32::MIN && sb == -1 {
                    sa as u32
                } else {
                    sa.wrapping_div(sb) as u32
                }
            }
            5 => a.checked_div(b).unwrap_or(u32::MAX),
            6 => {
                let sa = a as i32;
                let sb = b as i32;
                if sb == 0 {
                    a
                } else if sa == i32::MIN && sb == -1 {
                    0
                } else {
                    sa.wrapping_rem(sb) as u32
                }
            }
            7 => {
                if b == 0 {
                    a
                } else {
                    a % b
                }
            }
            _ => 0,
        }
    }

    fn exec_amo(&mut self, funct5: u8, funct3: u8, rd: u8, rs2: u8, pa: u64) {
        let src2 = self.regs[rs2 as usize];
        let is_word = funct3 == 2;

        match funct5 {
            0x02 => {
                // LR
                let val = if is_word {
                    self.phys_read(pa, 4) as i32 as i64 as u64
                } else {
                    self.phys_read(pa, 8)
                };
                self.set_rd(rd, val);
                self.reservation = Some(pa);
            }
            0x03 => {
                // SC
                if self.reservation == Some(pa) {
                    if is_word {
                        self.phys_write(pa, 4, src2);
                    } else {
                        self.phys_write(pa, 8, src2);
                    }
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
                if is_word {
                    self.phys_write(pa, 4, new_val);
                } else {
                    self.phys_write(pa, 8, new_val);
                }
                self.set_rd(rd, old);
            }
        }
    }
}

impl Drop for RiscvCpu {
    fn drop(&mut self) {
        if std::env::var_os("RHETSIM_JIT_STATS").is_none() {
            return;
        }
        let (compiles, executions, invalidations) =
            self.jit.as_ref().map(|j| j.stats()).unwrap_or((0, 0, 0));
        eprintln!(
            "jit-stats hart={} execs={} compiles={} invalidations={} \
             slow_loads={} slow_stores={} sfence={} satp={}",
            self.csr.mhartid,
            executions,
            compiles,
            invalidations,
            self.slow_loads,
            self.slow_stores,
            self.sfence_count,
            self.satp_writes
        );
    }
}

impl Unit for RiscvCpu {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "riscv-rv64-core"
    }
    fn caps(&self) -> UnitCaps {
        UnitCaps::EXECUTABLE
    }

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
        self.slow_loads = 0;
        self.slow_stores = 0;
        self.sfence_count = 0;
        self.satp_writes = 0;
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
        // Irq-line syncing is not free; a block only runs a handful of
        // instructions, so poll every 1024 instructions instead of per block.
        let mut next_irq_check = 0u64;
        while executed < budget_ns && !self.halted {
            if executed >= next_irq_check {
                self.check_interrupt();
                if self.halted {
                    break;
                }
                next_irq_check = executed + 1024;
            }

            let before = self.insn_count;
            let enter_pc = self.pc;
            let enter_priv = self.priv_mode;
            let used_jit = self.jit_step();
            if !used_jit {
                self.step_one_interp();
            }
            let ran = self.insn_count - before;
            self.csr.advance_counters(ran);
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
        let mut state = Vec::with_capacity(32 * 8 + 8 + 1 + 1 + csr_data.len() + 32 * 8);
        for r in &self.regs {
            state.extend_from_slice(&r.to_le_bytes());
        }
        state.extend_from_slice(&self.pc.to_le_bytes());
        state.push(self.halted as u8);
        state.push(self.priv_mode);
        state.extend_from_slice(&csr_data);
        for r in &self.fregs {
            state.extend_from_slice(&r.to_le_bytes());
        }
        Ok(state)
    }

    fn load_state(&mut self, data: &[u8]) -> OmniResult<()> {
        let min_size = 32 * 8 + 8 + 1;
        if data.len() < min_size {
            return Err(OmniError::SnapshotError("riscv state too short".into()));
        }
        for (i, reg) in self.regs.iter_mut().enumerate() {
            let off = i * 8;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&data[off..off + 8]);
            *reg = u64::from_le_bytes(buf);
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

        let fp_offset = csr_offset + CsrFile::state_size();
        if data.len() >= fp_offset + 32 * 8 {
            for (i, reg) in self.fregs.iter_mut().enumerate() {
                let off = fp_offset + i * 8;
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&data[off..off + 8]);
                *reg = u64::from_le_bytes(buf);
            }
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "rv64imac-interp"
    }
}

#[cfg(test)]
mod tests {
    // Instruction encodings are written with explicit zero fields for clarity.
    #![allow(clippy::identity_op)]

    use super::*;

    fn cpu_with(insns: &[u32]) -> RiscvCpu {
        let mut cpu = RiscvCpu::new("test", 4096, 0x8000_0000);
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu
    }

    fn fp_op(funct7: u32, funct3: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
        (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | 0x53
    }

    fn fp_fma(opcode: u32, funct7: u32, rd: u32, rs1: u32, rs2: u32, rs3: u32) -> u32 {
        (rs3 << 27) | (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (rd << 7) | opcode
    }

    fn bm_op(opcode: u32, funct7: u32, funct3: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
        (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
    }

    fn bm_imm(opcode: u32, funct3: u32, rd: u32, rs1: u32, funct12: u32) -> u32 {
        (funct12 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
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
        let mut cpu = cpu_with(&[0x00A00093, 0x00A00113, 0x00208463, 0x06400093, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 10);
    }

    #[test]
    fn test_branch_not_taken() {
        let mut cpu = cpu_with(&[0x00A00093, 0x01400113, 0x00208463, 0x06400093, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 100);
    }

    #[test]
    fn test_jal() {
        let mut cpu = cpu_with(&[0x008000EF, 0x06400093, 0x00000073]);
        let base = cpu.pc();
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), base + 4);
    }

    #[test]
    fn test_uart_output() {
        let uart_addr = 0x1000_0000u64;
        let mut cpu = RiscvCpu::new("test", 4096, 0x8000_0000);
        cpu.set_uart_addr(uart_addr);
        let insns: Vec<u32> = vec![0x10000137, 0x04800093, 0x00110023, 0x00000073];
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.exec(10).unwrap();
        assert_eq!(cpu.uart_output(), b"H");
    }

    #[test]
    fn test_save_load_state() {
        let mut cpu = cpu_with(&[0x02A00093, 0x00000073]);
        cpu.exec(10).unwrap();
        let state = cpu.save_state().unwrap();
        let mut cpu2 = RiscvCpu::new("test2", 4096, 0x8000_0000);
        cpu2.load_state(&state).unwrap();
        assert_eq!(cpu2.reg(1), 42);
        assert!(cpu2.is_halted());
    }

    #[test]
    fn test_loop_countdown() {
        let mut cpu = cpu_with(&[0x00500093, 0xFFF08093, 0xFE009EE3, 0x00000073]);
        cpu.exec(100).unwrap();
        assert_eq!(cpu.reg(1), 0);
        assert!(cpu.is_halted());
    }

    #[test]
    fn test_x0_always_zero() {
        let mut cpu = cpu_with(&[0x02A00013, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn test_bus_backed_read_write() {
        use rhetsim_core::mem::MemAttrs;

        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("dram0", 64 * 1024, 0));
        let mut bus = MemBus::new("sys-bus");
        bus.map(
            dram.clone() as Arc<dyn rhetsim_core::mem::MemTarget>,
            0,
            MemAttrs::rw(),
            None,
        )
        .unwrap();
        let bus = Arc::new(bus);

        let mut cpu = RiscvCpu::with_bus("rv-core0", bus.clone(), 0);
        let insns: Vec<u32> = vec![0x06300093, 0x10000113, 0x00112023, 0x00012183, 0x00000073];
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
        let mut cpu = cpu_with(&[0x06400113, csrrw, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(1), 0);
        assert_eq!(cpu.csr().mscratch, 100);
    }

    #[test]
    fn test_csrrs_mie() {
        let csrrs = (0x304 << 20) | (1 << 15) | (2 << 12) | (2 << 7) | 0x73;
        let mut cpu = cpu_with(&[0x08000093, csrrs, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.csr().mie, 128);
    }

    #[test]
    fn test_mul() {
        let mul = (0x01 << 25) | (2 << 20) | (1 << 15) | (0 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[0x00700093, 0x00600113, mul, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 42);
    }

    #[test]
    fn test_div() {
        let div = (0x01 << 25) | (2 << 20) | (1 << 15) | (4 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[0x02A00093, 0x00700113, div, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 6);
    }

    #[test]
    fn test_div_by_zero() {
        let div = (0x01 << 25) | (2 << 20) | (1 << 15) | (4 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[0x02A00093, 0x00000113, div, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), u64::MAX);
    }

    #[test]
    fn test_rem() {
        let rem = (0x01 << 25) | (2 << 20) | (1 << 15) | (6 << 12) | (3 << 7) | 0x33;
        let mut cpu = cpu_with(&[0x02B00093, 0x00700113, rem, 0x00000073]);
        cpu.exec(10).unwrap();
        assert_eq!(cpu.reg(3), 1);
    }

    #[test]
    fn test_lr_sc_w() {
        let mut cpu = RiscvCpu::new("test", 8192, 0);
        let sw = (0 << 25) | (1 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let lr_w = (0x02 << 27) | (0 << 20) | (2 << 15) | (2 << 12) | (3 << 7) | 0x2F;
        let sc_w = (0x03 << 27) | (4 << 20) | (2 << 15) | (2 << 12) | (5 << 7) | 0x2F;
        let lw_x6 = (0 << 20) | (2 << 15) | (2 << 12) | (6 << 7) | 0x03;
        let insns: Vec<u32> = vec![
            0x02A00093, 0x10000113, sw, lr_w, 0x06300213, sc_w, lw_x6, 0x00000073,
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
        let sw = (0 << 25) | (1 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x23;
        let amoadd_w = (0x00 << 27) | (4 << 20) | (2 << 15) | (2 << 12) | (3 << 7) | 0x2F;
        let lw_x5 = (0 << 20) | (2 << 15) | (2 << 12) | (5 << 7) | 0x03;
        let insns: Vec<u32> = vec![
            0x00A00093, 0x10000113, sw, 0x00500213, amoadd_w, lw_x5, 0x00000073,
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
        let csrrw_mtvec = (0x305 << 20) | (1 << 15) | (1 << 12) | (0 << 7) | 0x73;
        let csrrs_mie = (0x304 << 20) | (2 << 15) | (2 << 12) | (0 << 7) | 0x73;
        let csrrs_mstatus = (0x300 << 20) | (3 << 15) | (2 << 12) | (0 << 7) | 0x73;
        let main_code: Vec<u32> = vec![
            0x10000093,
            csrrw_mtvec,
            0x08000113,
            csrrs_mie,
            0x00800193,
            csrrs_mstatus,
            0x02A00593,
            0x00000073,
        ];
        for (i, insn) in main_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        let mret = 0x30200073u32;
        let csrrc_mie = (0x304u32 << 20) | (9 << 15) | (3 << 12) | (0 << 7) | 0x73;
        let handler: Vec<u32> = vec![0x00100513, 0x08000493, csrrc_mie, mret];
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
        // PMP default-denies S/U accesses without a matching entry, so
        // cover the whole address space like firmware would.
        cpu.csr_mut().pmpaddr[0] = pmp::PMPADDR_MASK;
        cpu.csr_mut().pmpcfg[0] = 0x1F; // NAPOT | R | W | X

        // mepc = 0x200 (S-mode entry)
        let csrrw_mepc = (0x341u32 << 20) | (1 << 15) | (1 << 12) | (0 << 7) | 0x73;
        // mstatus: set MPP to S-mode (01 << 11) and MIE
        let csrrw_mstatus = (0x300u32 << 20) | (2 << 15) | (1 << 12) | (0 << 7) | 0x73;
        // mtvec = 0x100 (M-mode trap handler)
        let csrrw_mtvec = (0x305u32 << 20) | (3 << 15) | (1 << 12) | (0 << 7) | 0x73;
        let mret = 0x30200073u32;

        let m_code: Vec<u32> = vec![
            0x20000093, // ADDI x1, x0, 0x200 (S-mode entry)
            csrrw_mepc, // CSRRW x0, mepc, x1
            0x00800113, // ADDI x2, x0, 0x008 (MIE bit)
            // MPP = S-mode (1 << 11 = 0x800)
            0x00001137,    // LUI x2, 0x1
            0x80010113,    // ADDI x2, x2, -0x800 → x2 = 0x800
            csrrw_mstatus, // CSRRW x0, mstatus, x2
            0x10000193,    // ADDI x3, x0, 0x100 (handler)
            csrrw_mtvec,   // CSRRW x0, mtvec, x3
            mret,          // MRET → S-mode at 0x200
        ];
        for (i, insn) in m_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }

        // S-mode code at 0x200
        let s_code: Vec<u32> = vec![
            0x00100513, // ADDI x10, x0, 1
            0x00000073, // ECALL → trap to M-mode (cause 9)
        ];
        for (i, insn) in s_code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x200 + i * 4) as u64);
        }

        // M-mode trap handler at 0x100
        let handler: Vec<u32> = vec![
            0x00100593, // ADDI x11, x0, 1
        ];
        // After this instruction, next is 0x104 which is 0 → halt
        for (i, insn) in handler.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x100 + i * 4) as u64);
        }

        cpu.set_pc(0);
        cpu.exec(50).unwrap();

        assert_eq!(cpu.reg(10), 1); // S-mode executed
        assert_eq!(cpu.reg(11), 1); // M-mode handler executed
        assert_eq!(cpu.csr().mcause, 9); // ecall from S-mode
    }

    // ── XThead vendor instructions ─────────────────────────────

    fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13
    }

    fn sd(rs2: u32, rs1: u32, imm: i32) -> u32 {
        let imm = imm as u32 & 0xFFF;
        ((imm >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (3 << 12) | ((imm & 0x1F) << 7) | 0x23
    }

    fn th_r(funct7: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
        (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (1 << 12) | (rd << 7) | 0x0B
    }

    fn th_ext_raw(funct3: u32, rd: u32, rs1: u32, msb: u32, lsb: u32) -> u32 {
        (msb << 26) | (lsb << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | 0x0B
    }

    fn th_inc(funct3: u32, hi5: u32, rd: u32, rs1: u32, imm5: i32, imm2: u32) -> u32 {
        (hi5 << 27)
            | ((imm2 & 0x3) << 25)
            | ((imm5 as u32 & 0x1F) << 20)
            | (rs1 << 15)
            | (funct3 << 12)
            | (rd << 7)
            | 0x0B
    }

    fn th_idx(funct3: u32, hi5: u32, rd: u32, rs1: u32, rs2: u32, imm2: u32) -> u32 {
        (hi5 << 27)
            | ((imm2 & 0x3) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (funct3 << 12)
            | (rd << 7)
            | 0x0B
    }

    fn th_pair(funct3: u32, hi5: u32, rd1: u32, rs: u32, rd2: u32, sh2: u32) -> u32 {
        (hi5 << 27)
            | ((sh2 & 0x3) << 25)
            | (rd2 << 20)
            | (rs << 15)
            | (funct3 << 12)
            | (rd1 << 7)
            | 0x0B
    }

    fn run_thead(insns: &[u32], setup: impl FnOnce(&mut RiscvCpu)) -> RiscvCpu {
        let mut cpu = RiscvCpu::new("thead", 8192, 0);
        setup(&mut cpu);
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        cpu.exec(64).unwrap();
        cpu
    }

    #[test]
    fn test_thead_addsl_and_condmov() {
        let cpu = run_thead(
            &[
                addi(1, 0, 5),
                addi(2, 0, 3),
                th_r(0x01, 3, 1, 2), // th.addsl1
                addi(4, 0, 0),
                th_r(0x20, 4, 1, 4), // th.mveqz (rs2 == 0)
                th_r(0x21, 4, 2, 1), // th.mvnez (rs2 != 0)
                0x00000073,
            ],
            |_| {},
        );
        assert_eq!(cpu.reg(3), 11);
        assert_eq!(cpu.reg(4), 3);
    }

    #[test]
    fn test_thead_extract() {
        let cpu = run_thead(
            &[
                0x123450B7,                 // LUI x1, 0x12345
                addi(1, 1, 0x6F8),          // x1 = 0x1234_56F8
                th_ext_raw(2, 2, 1, 7, 0),  // th.ext  x2, x1, 7, 0
                th_ext_raw(3, 3, 1, 7, 0),  // th.extu x3, x1, 7, 0
                th_ext_raw(2, 4, 1, 15, 8), // th.ext  x4, x1, 15, 8
                0x00000073,
            ],
            |_| {},
        );
        assert_eq!(cpu.reg(2), u64::MAX - 7); // 0xF8 sign-extended
        assert_eq!(cpu.reg(3), 0xF8);
        assert_eq!(cpu.reg(4), 0x56);
    }

    #[test]
    fn test_thead_mac() {
        let cpu = run_thead(
            &[
                addi(1, 0, 3),
                addi(2, 0, 4),
                addi(3, 0, 10),
                th_r(0x10, 3, 1, 2), // th.mula: 10 + 12
                th_r(0x11, 3, 1, 2), // th.muls: 22 - 12
                addi(4, 0, 1),
                th_r(0x12, 4, 1, 2), // th.mulaw: 1 + 12
                0x00000073,
            ],
            |_| {},
        );
        assert_eq!(cpu.reg(3), 10);
        assert_eq!(cpu.reg(4), 13);
    }

    #[test]
    fn test_thead_fmv_and_rev() {
        let cpu = run_thead(
            &[
                th_r(0x60, 1, 3, 0), // th.fmv.x.hw x1, f3
                addi(2, 0, 0x7AB),
                th_r(0x50, 3, 2, 0), // th.fmv.hw.x f3, x2
                addi(4, 0, 0),
                th_r(0x41, 5, 1, 0), // th.rev x5, x1
                0x00000073,
            ],
            |cpu| cpu.fregs[3] = 0x1234_5678_9ABC_DEF0,
        );
        assert_eq!(cpu.reg(1), 0x1234_5678);
        assert_eq!(cpu.fregs[3], 0x0000_07AB_9ABC_DEF0);
        assert_eq!(cpu.reg(5), 0x78563412_00000000);
    }

    #[test]
    fn test_thead_meminc() {
        let cpu = run_thead(
            &[
                sd(3, 1, 0),                 // SD x3, 0(x1)
                th_inc(4, 0x0F, 2, 1, 8, 0), // th.ldia x2, 8(x1)
                0x00000073,
            ],
            |cpu| {
                cpu.set_reg(1, 0x100);
                cpu.set_reg(3, 0x1122_3344_5566_7788);
            },
        );
        assert_eq!(cpu.reg(2), 0x1122_3344_5566_7788);
        assert_eq!(cpu.reg(1), 0x108);
    }

    #[test]
    fn test_thead_memidx() {
        let cpu = run_thead(
            &[
                th_idx(5, 0x0C, 3, 1, 2, 1), // th.srd x3, (x2 << 1)(x1)
                th_idx(4, 0x0C, 4, 1, 2, 1), // th.lrd x4, (x2 << 1)(x1)
                0x00000073,
            ],
            |cpu| {
                cpu.set_reg(1, 0x100);
                cpu.set_reg(2, 4);
                cpu.set_reg(3, 0xDEAD_BEEF_CAFE_1234);
            },
        );
        assert_eq!(cpu.reg(4), 0xDEAD_BEEF_CAFE_1234);
    }

    #[test]
    fn test_thead_mempair() {
        let cpu = run_thead(
            &[
                th_pair(5, 0x1F, 2, 1, 3, 1), // th.sdd x2, x3, 16(x1)
                th_pair(4, 0x1F, 2, 1, 3, 1), // th.ldd x2, x3, 16(x1)
                0x00000073,
            ],
            |cpu| {
                cpu.set_reg(1, 0x100);
                cpu.set_reg(2, 0x1111_2222_3333_4444);
                cpu.set_reg(3, 0x5555_6666_7777_8888);
            },
        );
        assert_eq!(cpu.reg(2), 0x1111_2222_3333_4444);
        assert_eq!(cpu.reg(3), 0x5555_6666_7777_8888);
    }

    #[test]
    fn test_thead_fmemidx() {
        let cpu = run_thead(
            &[
                th_idx(7, 0x08, 3, 1, 2, 2), // th.fsrw f3, (x2 << 2)(x1)
                th_idx(6, 0x08, 4, 1, 2, 2), // th.flrw f4, (x2 << 2)(x1)
                0x00000073,
            ],
            |cpu| {
                cpu.set_reg(1, 0x100);
                cpu.set_reg(2, 4);
                cpu.fregs[3] = 0xFFFF_FFFF_1122_3344;
            },
        );
        assert_eq!(cpu.fregs[4], 0xFFFF_FFFF_1122_3344);
    }

    #[test]
    fn test_thead_cmo_is_nop() {
        let cpu = run_thead(
            &[
                0x0010000B, // th.dcache.call
                addi(1, 0, 7),
                0x00000073,
            ],
            |_| {},
        );
        assert_eq!(cpu.reg(1), 7);
    }

    #[test]
    fn test_pmp_denies_s_mode_load() {
        let mut cpu = RiscvCpu::new("pmp", 0x10000, 0);
        // 4 KiB NAPOT RW region at 0x1000 for S/U.
        cpu.csr_mut().pmpaddr[0] = (0x1000 >> 2) | (4096 / 8 - 1);
        cpu.csr_mut().pmpcfg[0] = 0x1F;
        let code = [
            addi(1, 0, 0x200), // x1 = 0x200 (outside the PMP region)
            0x0000A103,        // LW x2, 0(x1) → load access fault
            0x00000073,
        ];
        for (i, insn) in code.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (0x1000 + i * 4) as u64);
        }
        cpu.set_pc(0x1000);
        cpu.set_priv_mode(csr::PRV_S);
        cpu.exec(16).unwrap();
        assert!(cpu.is_halted());
        assert_eq!(cpu.csr().mcause, 5);
        assert_eq!(cpu.csr().mtval, 0x200);
    }

    // ── JIT / interpreter parity ───────────────────────────────

    fn run_jit_parity_program(insns: &[u32], jit_enabled: bool) -> RiscvCpu {
        run_jit_parity_program_setup(insns, jit_enabled, |_| {})
    }

    fn run_jit_parity_program_setup(
        insns: &[u32],
        jit_enabled: bool,
        setup: impl FnOnce(&mut RiscvCpu),
    ) -> RiscvCpu {
        let mut cpu = RiscvCpu::new("parity", 8192, 0);
        cpu.set_jit_enabled(jit_enabled);
        for (i, insn) in insns.iter().enumerate() {
            cpu.load_binary(&insn.to_le_bytes(), (i * 4) as u64);
        }
        cpu.set_pc(0);
        setup(&mut cpu);
        cpu.exec(100_000).unwrap();
        cpu
    }

    fn assert_jit_parity(jit_cpu: &RiscvCpu, interp_cpu: &RiscvCpu) {
        for i in 0..32 {
            assert_eq!(jit_cpu.reg(i), interp_cpu.reg(i), "x{i} differs");
            assert_eq!(jit_cpu.fregs[i], interp_cpu.fregs[i], "f{i} differs");
        }
        assert_eq!(jit_cpu.pc(), interp_cpu.pc(), "pc differs");
        assert_eq!(
            jit_cpu.is_halted(),
            interp_cpu.is_halted(),
            "halted differs"
        );
        assert_eq!(
            jit_cpu.csr().mscratch,
            interp_cpu.csr().mscratch,
            "mscratch differs"
        );
        assert_eq!(jit_cpu.csr().mie, interp_cpu.csr().mie, "mie differs");
        assert_eq!(jit_cpu.csr().fcsr, interp_cpu.csr().fcsr, "fcsr differs");
        assert_eq!(
            jit_cpu.csr().mstatus & csr::MSTATUS_FS_MASK,
            interp_cpu.csr().mstatus & csr::MSTATUS_FS_MASK,
            "mstatus.FS differs"
        );
    }

    fn assert_jit_lowered(cpu: &RiscvCpu) {
        let jit = cpu.jit.as_ref().expect("JIT engine");
        assert_eq!(jit.fallback_count(), 0, "JIT fell back to the interpreter");
        assert!(jit.stats().0 > 0, "JIT compiled no blocks");
    }

    #[test]
    fn jit_interpreter_parity_loop_mem() {
        let insns = [
            0x1000_0113, // addi x2, x0, 0x100
            0x02A0_0193, // addi x3, x0, 42
            0x0031_2023, // sw   x3, 0(x2)
            0x0001_2203, // lw   x4, 0(x2)
            0x3E80_0093, // addi x1, x0, 1000
            0xFFF0_8093, // addi x1, x1, -1
            0xFE00_9EE3, // bne  x1, x0, -4
            0x0012_0293, // addi x5, x4, 1
            0x0000_0073, // ecall
        ];
        let jit_cpu = run_jit_parity_program(&insns, true);
        let interp_cpu = run_jit_parity_program(&insns, false);

        assert_eq!(jit_cpu.reg(1), 0);
        assert_eq!(jit_cpu.reg(2), 0x100);
        assert_eq!(jit_cpu.reg(3), 42);
        assert_eq!(jit_cpu.reg(4), 42);
        assert_eq!(jit_cpu.reg(5), 43);
        assert!(jit_cpu.is_halted());

        let (_, compiles, _) = jit_cpu.jit.as_ref().expect("JIT engine").stats();
        assert!(
            compiles > 0,
            "JIT compiled no blocks; parity test did not exercise JIT"
        );

        assert_jit_parity(&jit_cpu, &interp_cpu);
    }

    #[test]
    fn jit_interpreter_parity_jal() {
        let insns = [
            0x0080_00EF, // jal  x1, +8
            0x0010_0113, // addi x2, x0, 1 (skipped)
            0x0020_0193, // addi x3, x0, 2
            0x0000_0073, // ecall
        ];
        let jit_cpu = run_jit_parity_program(&insns, true);
        let interp_cpu = run_jit_parity_program(&insns, false);

        assert_eq!(jit_cpu.reg(1), 4);
        assert_eq!(jit_cpu.reg(2), 0);
        assert_eq!(jit_cpu.reg(3), 2);

        assert_jit_parity(&jit_cpu, &interp_cpu);
    }

    #[test]
    fn jit_interpreter_parity_csr() {
        let csrrw = (0x340u32 << 20) | (2 << 15) | (1 << 12) | (1 << 7) | 0x73;
        let insns = [
            0x0640_0113, // addi x2, x0, 100
            csrrw,       // csrrw x1, mscratch, x2
            0x0000_0073, // ecall
        ];
        let jit_cpu = run_jit_parity_program(&insns, true);
        let interp_cpu = run_jit_parity_program(&insns, false);

        assert_eq!(jit_cpu.reg(1), 0);
        assert_eq!(jit_cpu.csr().mscratch, 100);

        assert_jit_parity(&jit_cpu, &interp_cpu);
    }

    #[test]
    fn fp_double_arithmetic() {
        let mut cpu = cpu_with(&[
            fp_op(0x01, 0, 3, 1, 2), // FADD.D f3, f1, f2
            fp_op(0x09, 0, 4, 3, 2), // FMUL.D f4, f3, f2
            0x0000_0073,             // ECALL
        ]);
        cpu.set_fp64(1, 1.5);
        cpu.set_fp64(2, 2.25);
        cpu.exec(10).unwrap();

        assert_eq!(cpu.fp64(3), 3.75);
        assert_eq!(cpu.fp64(4), 8.4375);
        assert_ne!(cpu.csr().mstatus & csr::MSTATUS_FS_MASK, 0);
    }

    #[test]
    fn fp_convert_and_classify() {
        let mut cpu = cpu_with(&[
            0x02A0_0093,             // ADDI x1, x0, 42
            fp_op(0x69, 0, 2, 1, 0), // FCVT.D.W f2, x1
            fp_op(0x71, 1, 3, 2, 0), // FCLASS.D x3, f2
            0x0000_0073,             // ECALL
        ]);
        cpu.exec(10).unwrap();

        assert_eq!(cpu.fp64(2), 42.0);
        assert_eq!(cpu.reg(3), 1 << 6); // positive normal
    }

    #[test]
    fn fp_fused_multiply_add() {
        let mut cpu = cpu_with(&[
            fp_fma(0x43, 0x01, 3, 1, 2, 4), // FMADD.D f3, f1, f2, f4
            0x0000_0073,
        ]);
        cpu.set_fp64(1, 2.0);
        cpu.set_fp64(2, 3.0);
        cpu.set_fp64(4, 4.0);
        cpu.exec(10).unwrap();

        assert_eq!(cpu.fp64(3), 10.0);
    }

    #[test]
    fn bitmanip_zba_zbb() {
        let mut cpu = cpu_with(&[
            0x00A0_0093,                   // ADDI x1, x0, 10
            0x0140_0113,                   // ADDI x2, x0, 20
            bm_op(0x33, 0x10, 2, 3, 1, 2), // SH1ADD x3, x1, x2
            bm_op(0x33, 0x20, 7, 4, 1, 2), // ANDN x4, x1, x2
            bm_op(0x33, 0x05, 4, 5, 1, 2), // MIN x5, x1, x2
            bm_imm(0x13, 1, 6, 1, 0x600),  // CLZ x6, x1
            bm_imm(0x13, 5, 7, 1, 0x601),  // RORI x7, x1, 1
            0x0000_0073,                   // ECALL
        ]);
        cpu.exec(20).unwrap();

        assert_eq!(cpu.reg(3), 40);
        assert_eq!(cpu.reg(4), 10);
        assert_eq!(cpu.reg(5), 10);
        assert_eq!(cpu.reg(6), 60);
        assert_eq!(cpu.reg(7), 5);
    }

    #[test]
    fn bitmanip_word_ops() {
        let mut cpu = cpu_with(&[
            bm_op(0x3B, 0x04, 0, 3, 1, 2), // ADD.UW x3, x1, x2
            bm_imm(0x1B, 1, 4, 1, 0x600),  // CLZW x4, x1
            0x0000_0073,
        ]);
        cpu.set_reg(1, 0xFFFF_FFFF_0000_0001);
        cpu.set_reg(2, 2);
        cpu.exec(10).unwrap();

        assert_eq!(cpu.reg(3), 3);
        assert_eq!(cpu.reg(4), 31);
    }

    #[test]
    fn bitmanip_zbc_zbs() {
        let mut cpu = cpu_with(&[
            0x0030_0093,                      // ADDI x1, x0, 3
            0x0050_0113,                      // ADDI x2, x0, 5
            bm_op(0x33, 0x05, 1, 3, 1, 2),    // CLMUL x3, x1, x2
            bm_op(0x33, 0x14, 1, 4, 0, 2),    // BSET x4, x0, x2
            bm_op(0x33, 0x24, 1, 5, 4, 2),    // BCLR x5, x4, x2
            bm_op(0x33, 0x34, 1, 6, 4, 2),    // BINV x6, x4, x2
            bm_op(0x33, 0x24, 5, 7, 4, 2),    // BEXT x7, x4, x2
            bm_imm(0x13, 1, 8, 0, 0x280 | 7), // BSETI x8, x0, 7
            bm_imm(0x13, 5, 9, 8, 0x480 | 7), // BEXTI x9, x8, 7
            0x0000_0073,
        ]);
        cpu.exec(20).unwrap();

        assert_eq!(cpu.reg(3), 0xF); // 0b11 * 0b101 without carries
        assert_eq!(cpu.reg(4), 1 << 5);
        assert_eq!(cpu.reg(5), 0);
        assert_eq!(cpu.reg(6), 0); // BINV toggles the bit set by BSET
        assert_eq!(cpu.reg(7), 1);
        assert_eq!(cpu.reg(8), 1 << 7);
        assert_eq!(cpu.reg(9), 1);
    }

    #[test]
    fn thead_sync_is_a_noop() {
        let mut cpu = cpu_with(&[0x0180_000B, 0x0000_0073]);
        cpu.exec(10).unwrap();
        assert!(cpu.is_halted());
    }

    #[test]
    fn compressed_instruction_traps_without_c_extension() {
        let mut cpu = RiscvCpu::new_with_type("test", 4096, 0, RiscvCpuType::RhetSimRv64Ima);
        cpu.load_binary(&0x0001u16.to_le_bytes(), 0); // C.NOP
        cpu.set_pc(0);
        cpu.set_jit_enabled(false);
        cpu.exec(1).unwrap();

        assert_eq!(cpu.csr().mcause, 2); // illegal instruction
        assert_eq!(cpu.csr().misa & (1 << 2), 0); // C is not advertised
    }

    // ── JIT lowering parity: F/D, bitmanip and CMO ─────────────

    fn fp_load(funct3: u32, rd: u32, rs1: u32, offset: i32) -> u32 {
        ((offset as u32) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | 0x07
    }

    fn fp_store(funct3: u32, rs1: u32, rs2: u32, offset: i32) -> u32 {
        let off = offset as u32;
        ((off >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | ((off & 0x1F) << 7) | 0x27
    }

    fn cbo(op: u32, rs1: u32) -> u32 {
        (op << 20) | (rs1 << 15) | (2 << 12) | 0x0F
    }

    #[test]
    fn jit_interpreter_parity_fp_arith() {
        let insns = [
            fp_op(0x01, 0, 3, 1, 2),    // fadd.d f3, f1, f2
            fp_op(0x05, 0, 4, 3, 2),    // fsub.d f4, f3, f2
            fp_op(0x09, 0, 5, 4, 2),    // fmul.d f5, f4, f2
            fp_op(0x0D, 0, 6, 5, 2),    // fdiv.d f6, f5, f2
            fp_op(0x0D, 0, 7, 6, 0),    // fdiv.d f7, f6, f0 (DZ)
            fp_op(0x0D, 0, 26, 0, 0),   // fdiv.d f26, f0, f0 (NV)
            fp_op(0x2D, 0, 8, 6, 0),    // fsqrt.d f8, f6
            fp_op(0x2D, 0, 28, 27, 0),  // fsqrt.d f28, f27 (negative, NV)
            fp_op(0x15, 1, 9, 1, 2),    // fmax.d f9, f1, f2
            fp_op(0x15, 0, 10, 1, 2),   // fmin.d f10, f1, f2
            fp_op(0x15, 1, 21, 21, 22), // fmax.d f21, NaN, NaN
            fp_op(0x15, 0, 25, 23, 24), // fmin.d f25, 1.0, NaN
            fp_op(0x11, 0, 11, 1, 2),   // fsgnj.d f11, f1, f2
            fp_op(0x11, 1, 12, 1, 2),   // fsgnjn.d f12, f1, f2
            fp_op(0x11, 2, 13, 1, 2),   // fsgnjx.d f13, f1, f2
            fp_op(0x51, 2, 14, 1, 2),   // feq.d x14, f1, f2
            fp_op(0x51, 1, 15, 1, 2),   // flt.d x15, f1, f2
            fp_op(0x51, 0, 16, 1, 2),   // fle.d x16, f1, f2
            fp_op(0x21, 0, 19, 18, 0),  // fcvt.d.s f19, f18
            fp_op(0x20, 0, 20, 19, 0),  // fcvt.s.d f20, f19
            0x0000_0073,                // ecall
        ];
        let setup = |cpu: &mut RiscvCpu| {
            cpu.set_fp64(1, 1.5);
            cpu.set_fp64(2, 2.75);
            cpu.set_fp64(21, f64::NAN);
            cpu.set_fp64(22, f64::NAN);
            cpu.set_fp64(23, 1.0);
            cpu.set_fp64(24, f64::NAN);
            cpu.set_fp64(27, -4.0);
            cpu.set_fp32(18, -2.5);
        };
        let jit_cpu = run_jit_parity_program_setup(&insns, true, setup);
        let interp_cpu = run_jit_parity_program_setup(&insns, false, setup);
        assert_jit_lowered(&jit_cpu);
        assert_jit_parity(&jit_cpu, &interp_cpu);

        assert_eq!(jit_cpu.fp64(3), 4.25);
        assert_eq!(jit_cpu.fp64(9), 2.75);
        assert_eq!(jit_cpu.fp64(10), 1.5);
        assert_eq!(jit_cpu.reg(14), 0);
        assert_eq!(jit_cpu.reg(15), 1);
        assert_eq!(jit_cpu.reg(16), 1);
        assert_eq!(jit_cpu.fp64(19), -2.5);
        assert_eq!(jit_cpu.fp32(20), -2.5);
        assert_eq!(jit_cpu.fp64(21).to_bits(), 0x7FF8_0000_0000_0000);
        assert_eq!(jit_cpu.fp64(25), 1.0);
        assert!(jit_cpu.fp64(7).is_infinite());
        assert!(jit_cpu.fp64(26).is_nan());
        assert!(jit_cpu.fp64(28).is_nan());
        assert_ne!(jit_cpu.csr().fcsr & (1 << 3), 0, "DZ flag missing");
        assert_ne!(jit_cpu.csr().fcsr & (1 << 4), 0, "NV flag missing");
    }

    #[test]
    fn jit_interpreter_parity_fp_convert() {
        let insns = [
            0xFF60_0093,               // addi x1, x0, -10
            fp_op(0x69, 0, 2, 1, 0),   // fcvt.d.w f2, x1
            fp_op(0x68, 0, 3, 1, 0),   // fcvt.s.w f3, x1
            fp_op(0x61, 0, 4, 2, 0),   // fcvt.w.d x4, f2
            fp_op(0x61, 0, 5, 2, 1),   // fcvt.wu.d x5, f2
            fp_op(0x61, 0, 6, 2, 2),   // fcvt.l.d x6, f2
            fp_op(0x61, 0, 7, 2, 3),   // fcvt.lu.d x7, f2
            fp_op(0x71, 0, 8, 2, 0),   // fmv.x.d x8, f2
            fp_op(0x71, 1, 9, 2, 0),   // fclass.d x9, f2
            fp_op(0x79, 0, 10, 8, 0),  // fmv.d.x f10, x8
            fp_op(0x70, 1, 11, 3, 0),  // fclass.s x11, f3
            fp_op(0x70, 0, 12, 3, 0),  // fmv.x.w x12, f3
            fp_op(0x78, 0, 13, 1, 0),  // fmv.w.x f13, x1
            fp_op(0x61, 0, 14, 21, 0), // fcvt.w.d x14, f21 (NaN)
            fp_op(0x71, 1, 15, 21, 0), // fclass.d x15, f21
            fp_op(0x61, 0, 16, 20, 0), // fcvt.w.d 3e9 (saturate)
            fp_op(0x61, 0, 17, 20, 1), // fcvt.wu.d 3e9 (saturate)
            fp_op(0x61, 0, 18, 23, 2), // fcvt.l.d 1e300 (saturate)
            fp_op(0x61, 0, 19, 23, 3), // fcvt.lu.d 1e300 (saturate)
            fp_op(0x61, 0, 24, 22, 1), // fcvt.wu.d 5e9 (saturate)
            0x0000_0073,               // ecall
        ];
        let setup = |cpu: &mut RiscvCpu| {
            cpu.set_fp64(21, f64::NAN);
            cpu.set_fp64(20, 3.0e9);
            cpu.set_fp64(22, 5.0e9);
            cpu.set_fp64(23, 1.0e300);
        };
        let jit_cpu = run_jit_parity_program_setup(&insns, true, setup);
        let interp_cpu = run_jit_parity_program_setup(&insns, false, setup);
        assert_jit_lowered(&jit_cpu);
        assert_jit_parity(&jit_cpu, &interp_cpu);

        assert_eq!(jit_cpu.fp64(2), -10.0);
        assert_eq!(jit_cpu.fp32(3), -10.0);
        assert_eq!(jit_cpu.reg(4) as i64, -10);
        assert_eq!(jit_cpu.reg(5), 0);
        assert_eq!(jit_cpu.reg(6) as i64, -10);
        assert_eq!(jit_cpu.reg(7), 0);
        assert_eq!(jit_cpu.reg(8), (-10.0f64).to_bits());
        assert_eq!(jit_cpu.reg(9), 1 << 1); // negative normal
        assert_eq!(jit_cpu.fp64(10).to_bits(), (-10.0f64).to_bits());
        assert_eq!(jit_cpu.reg(11), 1 << 1);
        assert_eq!(jit_cpu.reg(12), 0xFFFF_FFFF_C120_0000);
        assert_eq!(jit_cpu.fregs[13], 0xFFFF_FFFF_FFFF_FFF6);
        assert_eq!(jit_cpu.reg(14), 0);
        assert_eq!(jit_cpu.reg(15), 1 << 9); // quiet NaN
        assert_eq!(jit_cpu.reg(16), i32::MAX as u64);
        assert_eq!(jit_cpu.reg(17), 3_000_000_000);
        assert_eq!(jit_cpu.reg(18), i64::MAX as u64);
        assert_eq!(jit_cpu.reg(19), u64::MAX);
        assert_eq!(jit_cpu.reg(24), u32::MAX as u64);
        assert_ne!(jit_cpu.csr().fcsr & (1 << 4), 0, "NV flag missing");
    }

    #[test]
    fn jit_interpreter_parity_fp_mem() {
        let insns = [
            0x1000_0093,           // addi x1, x0, 0x100
            fp_store(3, 1, 2, 0),  // fsd f2, 0(x1)
            fp_load(3, 3, 1, 0),   // fld f3, 0(x1)
            fp_store(2, 1, 5, 8),  // fsw f5, 8(x1)
            fp_load(2, 4, 1, 8),   // flw f4, 8(x1)
            fp_store(3, 1, 6, 16), // fsd f6, 16(x1)
            fp_load(3, 7, 1, 16),  // fld f7, 16(x1)
            0x0000_B203,           // ld x4, 0(x1)
            0x0080_B183,           // ld x3, 8(x1)
            0x0000_0073,           // ecall
        ];
        let setup = |cpu: &mut RiscvCpu| {
            cpu.set_fp64(2, -123.456);
            cpu.set_fp32(5, 3.5);
            cpu.set_fp64(6, 1.0e300);
        };
        let jit_cpu = run_jit_parity_program_setup(&insns, true, setup);
        let interp_cpu = run_jit_parity_program_setup(&insns, false, setup);
        assert_jit_lowered(&jit_cpu);
        assert_jit_parity(&jit_cpu, &interp_cpu);

        assert_eq!(jit_cpu.fp64(3), -123.456);
        assert_eq!(jit_cpu.fp32(4), 3.5);
        assert_eq!(jit_cpu.fp64(7), 1.0e300);
        assert_eq!(jit_cpu.reg(4), (-123.456f64).to_bits());
        assert_eq!(jit_cpu.reg(3) as u32, 3.5f32.to_bits());
    }

    #[test]
    fn jit_interpreter_parity_fp_fma() {
        let insns = [
            fp_fma(0x43, 0x01, 3, 1, 2, 4),    // fmadd.d
            fp_fma(0x47, 0x01, 5, 1, 2, 4),    // fmsub.d
            fp_fma(0x4B, 0x01, 6, 1, 2, 4),    // fnmsub.d
            fp_fma(0x4F, 0x01, 7, 1, 2, 4),    // fnmadd.d
            fp_fma(0x43, 0x00, 8, 11, 12, 14), // fmadd.s
            0x0000_0073,                       // ecall
        ];
        let setup = |cpu: &mut RiscvCpu| {
            cpu.set_fp64(1, 2.0);
            cpu.set_fp64(2, 3.0);
            cpu.set_fp64(4, 4.0);
            cpu.set_fp32(11, 2.0);
            cpu.set_fp32(12, 3.0);
            cpu.set_fp32(14, 4.0);
        };
        let jit_cpu = run_jit_parity_program_setup(&insns, true, setup);
        let interp_cpu = run_jit_parity_program_setup(&insns, false, setup);
        assert_jit_lowered(&jit_cpu);
        assert_jit_parity(&jit_cpu, &interp_cpu);

        assert_eq!(jit_cpu.fp64(3), 10.0);
        assert_eq!(jit_cpu.fp64(5), 2.0);
        assert_eq!(jit_cpu.fp64(6), -2.0);
        assert_eq!(jit_cpu.fp64(7), -10.0);
        assert_eq!(jit_cpu.fp32(8), 10.0);
    }

    #[test]
    fn jit_interpreter_parity_bitmanip() {
        let insns = [
            bm_op(0x33, 0x10, 2, 10, 1, 2), // sh1add
            bm_op(0x33, 0x10, 4, 11, 1, 2), // sh2add
            bm_op(0x33, 0x10, 6, 12, 1, 2), // sh3add
            bm_op(0x33, 0x20, 4, 13, 1, 2), // xnor
            bm_op(0x33, 0x20, 6, 14, 1, 2), // orn
            bm_op(0x33, 0x20, 7, 15, 1, 2), // andn
            bm_op(0x33, 0x05, 4, 16, 1, 2), // min
            bm_op(0x33, 0x05, 5, 17, 1, 2), // minu
            bm_op(0x33, 0x05, 6, 18, 1, 2), // max
            bm_op(0x33, 0x05, 7, 19, 1, 2), // maxu
            bm_op(0x33, 0x14, 1, 20, 1, 2), // bset
            bm_op(0x33, 0x24, 1, 21, 1, 2), // bclr
            bm_op(0x33, 0x24, 5, 22, 1, 2), // bext
            bm_op(0x33, 0x34, 1, 23, 1, 2), // binv
            bm_op(0x33, 0x30, 1, 24, 1, 2), // rol
            bm_op(0x33, 0x30, 5, 25, 1, 2), // ror
            bm_op(0x33, 0x05, 1, 26, 1, 2), // clmul
            bm_op(0x33, 0x05, 2, 27, 1, 2), // clmulr
            bm_op(0x33, 0x05, 3, 28, 1, 2), // clmulh
            bm_op(0x3B, 0x04, 0, 29, 1, 2), // add.uw
            bm_op(0x3B, 0x04, 4, 30, 1, 2), // zext.h
            bm_op(0x3B, 0x10, 2, 31, 1, 2), // sh1add.uw
            bm_op(0x3B, 0x30, 1, 5, 1, 3),  // rolw
            bm_op(0x3B, 0x30, 5, 6, 1, 3),  // rorw
            bm_imm(0x13, 1, 7, 1, 0x600),   // clz
            bm_imm(0x13, 1, 8, 1, 0x601),   // ctz
            bm_imm(0x13, 1, 9, 1, 0x602),   // cpop
            bm_imm(0x13, 1, 10, 1, 0x604),  // sext.b
            bm_imm(0x13, 1, 11, 1, 0x605),  // sext.h
            bm_imm(0x13, 5, 12, 1, 0x6B8),  // rev8
            bm_imm(0x13, 5, 13, 1, 0x287),  // orc.b
            bm_imm(0x13, 5, 14, 1, 0x603),  // rori 3
            bm_imm(0x13, 5, 15, 1, 0x485),  // bexti 5
            bm_imm(0x13, 1, 16, 1, 0x282),  // bseti 2
            bm_imm(0x13, 1, 17, 1, 0x485),  // bclri 5
            bm_imm(0x13, 1, 18, 1, 0x683),  // binvi 3
            bm_imm(0x1B, 1, 19, 4, 0x600),  // clzw
            bm_imm(0x1B, 1, 20, 4, 0x601),  // ctzw
            bm_imm(0x1B, 1, 21, 4, 0x602),  // cpopw
            bm_imm(0x1B, 5, 22, 4, 0x603),  // roriw 3
            bm_imm(0x1B, 1, 23, 4, 0x087),  // slli.uw 7
            0x0000_0073,                    // ecall
        ];
        let setup = |cpu: &mut RiscvCpu| {
            cpu.set_reg(1, 0x0123_4567_89AB_CDEF);
            cpu.set_reg(2, 0xF0F0_0F0F_AAAA_5555);
            cpu.set_reg(3, 37);
            cpu.set_reg(4, 1);
        };
        let jit_cpu = run_jit_parity_program_setup(&insns, true, setup);
        let interp_cpu = run_jit_parity_program_setup(&insns, false, setup);
        assert_jit_lowered(&jit_cpu);
        assert_jit_parity(&jit_cpu, &interp_cpu);

        let a = 0x0123_4567_89AB_CDEFu64;
        let b = 0xF0F0_0F0F_AAAA_5555u64;
        assert_eq!(jit_cpu.reg(24), a.rotate_left((b & 63) as u32));
        assert_eq!(jit_cpu.reg(25), a.rotate_right((b & 63) as u32));
        let (lo, hi) = RiscvCpu::clmul(a, b);
        assert_eq!(jit_cpu.reg(26), lo);
        assert_eq!(jit_cpu.reg(27), (lo >> 63) | (hi << 1));
        assert_eq!(jit_cpu.reg(28), hi);
        assert_eq!(jit_cpu.reg(12), a.swap_bytes());
        assert_eq!(jit_cpu.reg(19), 31); // clzw 1
        assert_eq!(jit_cpu.reg(20), 0); // ctzw 1
        assert_eq!(jit_cpu.reg(21), 1); // cpopw 1
        assert_eq!(jit_cpu.reg(22), 0x2000_0000); // roriw 3
        assert_eq!(jit_cpu.reg(23), 0x80); // slli.uw 7
        assert_eq!(jit_cpu.reg(29), (a as u32 as u64).wrapping_add(b));
        assert_eq!(jit_cpu.reg(30), a & 0xFFFF);
        assert_eq!(
            jit_cpu.reg(31),
            (a as u32 as u64).wrapping_shl(1).wrapping_add(b)
        );
    }

    #[test]
    fn jit_interpreter_parity_cbo() {
        let insns = [
            0x1000_0093, // addi x1, x0, 0x100
            0x05A0_0113, // addi x2, x0, 0x5A
            0x0020_B023, // sd x2, 0(x1)
            cbo(4, 1),   // cbo.zero x1
            0x0000_B183, // ld x3, 0(x1)
            0x0380_B203, // ld x4, 56(x1)
            cbo(1, 1),   // cbo.clean (no-op)
            cbo(2, 1),   // cbo.flush (no-op)
            cbo(0, 1),   // cbo.inval (no-op)
            0x0000_0073, // ecall
        ];
        let jit_cpu = run_jit_parity_program(&insns, true);
        let interp_cpu = run_jit_parity_program(&insns, false);
        assert_jit_lowered(&jit_cpu);
        assert_jit_parity(&jit_cpu, &interp_cpu);

        assert_eq!(jit_cpu.reg(3), 0);
        assert_eq!(jit_cpu.reg(4), 0);
    }
}
