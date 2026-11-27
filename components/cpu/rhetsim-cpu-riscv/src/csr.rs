//! RISC-V CSR file — M-mode and S-mode

use crate::pmp;
use crate::profile::RiscvCpuType;

// ── CSR addresses ─────────────────────────────────────────────

// S-mode
pub const SSTATUS: u16 = 0x100;
pub const SIE: u16 = 0x104;
pub const STVEC: u16 = 0x105;
pub const SSCRATCH: u16 = 0x140;
pub const SEPC: u16 = 0x141;
pub const SCAUSE: u16 = 0x142;
pub const STVAL: u16 = 0x143;
pub const SIP: u16 = 0x144;
pub const SATP: u16 = 0x180;
/// Sstc supervisor timer compare (RV64: 64-bit CSR).
pub const STIMECMP: u16 = 0x14D;

// AIA CSRs. We do not implement AIA; these reads must trap so OpenSBI's
// feature probe does not enable the AIA interrupt path (which would read
// MTOPI and silently skip MSIP processing).
pub const STOPI: u16 = 0xDB0;
pub const MTOPI: u16 = 0xFB0;

// M-mode
pub const MSTATUS: u16 = 0x300;
pub const MISA: u16 = 0x301;
pub const MEDELEG: u16 = 0x302;
pub const MIDELEG: u16 = 0x303;
pub const MIE: u16 = 0x304;
pub const MTVEC: u16 = 0x305;
pub const MCOUNTEREN: u16 = 0x306;
pub const MSCRATCH: u16 = 0x340;
pub const MEPC: u16 = 0x341;
pub const MCAUSE: u16 = 0x342;
pub const MTVAL: u16 = 0x343;
pub const MIP: u16 = 0x344;
pub const MCOUNTINHIBIT: u16 = 0x320;
pub const MHPMEVENT3: u16 = 0x323;
pub const MHPMCOUNTER3: u16 = 0xB03;

// PMP / Smepmp
pub const PMPCFG0: u16 = 0x3A0;
pub const PMPCFG1: u16 = 0x3A1;
pub const PMPCFG2: u16 = 0x3A2;
pub const PMPCFG3: u16 = 0x3A3;
pub const PMPADDR0: u16 = 0x3B0;
pub const MSECCFG: u16 = 0x747;
pub const MSECCFGH: u16 = 0x757;

// Counters (M-mode)
pub const MCYCLE: u16 = 0xB00;
pub const MINSTRET: u16 = 0xB02;

// Counters (S/U-mode readable)
pub const CYCLE: u16 = 0xC00;
pub const TIME: u16 = 0xC01;
pub const INSTRET: u16 = 0xC02;

// Machine info (read-only)
pub const MVENDORID: u16 = 0xF11;
pub const MARCHID: u16 = 0xF12;
pub const MIMPID: u16 = 0xF13;
pub const MHARTID: u16 = 0xF14;

// S-mode counter enable
pub const SCOUNTEREN: u16 = 0x106;
/// Sscofpmf: supervisor counter-overflow view.
pub const SCOUNTOVF: u16 = 0xDA0;

// Floating-point CSRs
pub const FFLAGS: u16 = 0x001;
pub const FRM: u16 = 0x002;
pub const FCSR: u16 = 0x003;

// ── mstatus bit positions ─────────────────────────────────────

pub const MSTATUS_SIE: u64 = 1 << 1;
pub const MSTATUS_MIE: u64 = 1 << 3;
pub const MSTATUS_SPIE: u64 = 1 << 5;
pub const MSTATUS_MPIE: u64 = 1 << 7;
pub const MSTATUS_SPP: u64 = 1 << 8;
pub const MSTATUS_MPP_MASK: u64 = 3 << 11;
pub const MSTATUS_MPP_M: u64 = 3 << 11;
pub const MSTATUS_FS_MASK: u64 = 3 << 13;
pub const MSTATUS_FS_DIRTY: u64 = 3 << 13;
pub const MSTATUS_MPRV: u64 = 1 << 17;
pub const MSTATUS_SUM: u64 = 1 << 18;
pub const MSTATUS_MXR: u64 = 1 << 19;
pub const MSTATUS_TVM: u64 = 1 << 20;
pub const MSTATUS_TW: u64 = 1 << 21;
pub const MSTATUS_TSR: u64 = 1 << 22;
/// mstatus.UXL (RV64): 2 = 64-bit user XLEN.
pub const MSTATUS_UXL_MASK: u64 = 3 << 32;
pub const MSTATUS_UXL_64: u64 = 2 << 32;
/// mstatus.SXL (RV64): 2 = 64-bit supervisor XLEN (read-only).
pub const MSTATUS_SXL_MASK: u64 = 3 << 34;
pub const MSTATUS_SXL_64: u64 = 2 << 34;

const MSTATUS_WRITABLE: u64 = MSTATUS_SIE
    | MSTATUS_MIE
    | MSTATUS_SPIE
    | MSTATUS_MPIE
    | MSTATUS_SPP
    | MSTATUS_MPP_MASK
    | MSTATUS_FS_MASK
    | MSTATUS_MPRV
    | MSTATUS_SUM
    | MSTATUS_MXR
    | MSTATUS_TVM
    | MSTATUS_TW
    | MSTATUS_TSR;

const SSTATUS_MASK: u64 = MSTATUS_SIE
    | MSTATUS_SPIE
    | MSTATUS_SPP
    | (3 << 13)
    | (3 << 15)
    | MSTATUS_SUM
    | MSTATUS_MXR
    | MSTATUS_UXL_MASK
    | (1u64 << 63);

const SSTATUS_WRITABLE: u64 =
    MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_FS_MASK | MSTATUS_SUM | MSTATUS_MXR;

// ── mie/mip bit positions ─────────────────────────────────────

pub const MIP_SSIP: u64 = 1 << 1;
pub const MIP_MSIP: u64 = 1 << 3;
pub const MIP_STIP: u64 = 1 << 5;
pub const MIP_MTIP: u64 = 1 << 7;
pub const MIP_SEIP: u64 = 1 << 9;
pub const MIP_MEIP: u64 = 1 << 11;
/// Sscofpmf local counter overflow interrupt.
pub const MIP_LCOFI: u64 = 1 << 13;

const SIE_MASK: u64 = MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_LCOFI;
const MIE_ALL: u64 = MIP_SSIP | MIP_MSIP | MIP_STIP | MIP_MTIP | MIP_SEIP | MIP_MEIP | MIP_LCOFI;

/// M-mode can write software-injected interrupt bits:
/// - MSIP: software interrupt (CLINT/rv-virt)
/// - SSIP/STIP/SEIP: needed by OpenSBI to inject S-mode IPI/timer/external
///
/// MEIP/MTIP are driven by the interrupt controllers (PLIC/CLINT) and stay
/// read-only; the CPU syncs them from IrqLineSource.
const MIP_WRITABLE: u64 = MIP_MSIP | MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_LCOFI;

// ── mcause ────────────────────────────────────────────────────

pub const MCAUSE_INTERRUPT: u64 = 1u64 << 63;

// ── Privilege modes ───────────────────────────────────────────

pub const PRV_U: u8 = 0;
pub const PRV_S: u8 = 1;
pub const PRV_M: u8 = 3;

// ── CsrFile ───────────────────────────────────────────────────

pub struct CsrFile {
    // Floating-point control and status
    pub fcsr: u32,
    // M-mode
    pub mstatus: u64,
    pub mie: u64,
    pub mip: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub mhartid: u64,
    pub mcycle: u64,
    pub minstret: u64,
    pub mcountinhibit: u64,
    /// Sscofpmf event selectors and counters (only 3..31 are meaningful).
    pub mhpmevent: [u64; 32],
    pub mhpmcounter: [u64; 32],
    /// `misa` derived from the CPU profile's implemented extensions.
    pub misa: u64,
    pub mvendorid: u64,
    pub marchid: u64,
    pub mimpid: u64,
    pub medeleg: u64,
    pub mideleg: u64,
    pub mcounteren: u64,
    // S-mode
    pub stvec: u64,
    pub sscratch: u64,
    pub sepc: u64,
    pub scause: u64,
    pub stval: u64,
    pub satp: u64,
    /// Sstc timer compare; when active, STIP is derived from
    /// `insn_count >= stimecmp` (see RiscvCpu::sync_irq_lines).
    pub stimecmp: u64,
    pub sstc_active: bool,
    pub scounteren: u64,
    // PMP / Smepmp
    pub pmpcfg: [u64; 2],
    pub pmpaddr: [u64; 16],
    /// Smepmp machine security configuration (`MML`/`MMWP`/`RLB`).
    pub mseccfg: u64,
}

impl CsrFile {
    pub fn new(hart_id: u64) -> Self {
        Self::new_with_type(hart_id, RiscvCpuType::default())
    }

    pub fn new_with_type(hart_id: u64, cpu_type: RiscvCpuType) -> Self {
        let profile = cpu_type.profile();
        Self {
            fcsr: 0,
            mstatus: MSTATUS_UXL_64 | MSTATUS_SXL_64,
            mie: 0,
            mip: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mhartid: hart_id,
            mcycle: 0,
            minstret: 0,
            mcountinhibit: 0,
            mhpmevent: [0; 32],
            mhpmcounter: [0; 32],
            misa: profile.misa(),
            mvendorid: profile.mvendorid,
            marchid: profile.marchid,
            mimpid: profile.mimpid,
            medeleg: 0,
            mideleg: 0,
            mcounteren: 0,
            stvec: 0,
            sscratch: 0,
            sepc: 0,
            scause: 0,
            stval: 0,
            satp: 0,
            stimecmp: u64::MAX,
            sstc_active: false,
            scounteren: 0,
            pmpcfg: [0; 2],
            pmpaddr: [0; 16],
            mseccfg: 0,
        }
    }

    pub fn read(&self, addr: u16, insn_count: u64) -> Option<u64> {
        match addr {
            FFLAGS => Some((self.fcsr & 0x1F) as u64),
            FRM => Some(((self.fcsr >> 5) & 0x7) as u64),
            FCSR => Some((self.fcsr & 0xFF) as u64),
            MSTATUS => Some(self.mstatus),
            MISA => Some(self.misa),
            MEDELEG => Some(self.medeleg),
            MIDELEG => Some(self.mideleg),
            MIE => Some(self.mie),
            MTVEC => Some(self.mtvec),
            MCOUNTEREN => Some(self.mcounteren),
            MSCRATCH => Some(self.mscratch),
            MEPC => Some(self.mepc),
            MCAUSE => Some(self.mcause),
            MTVAL => Some(self.mtval),
            MIP => Some(self.mip),
            MCOUNTINHIBIT => Some(self.mcountinhibit),
            MCYCLE => Some(self.mcycle),
            MINSTRET => Some(self.minstret),
            a @ MHPMEVENT3..=0x33F => Some(self.mhpmevent[(a - MHPMEVENT3 + 3) as usize]),
            a @ MHPMCOUNTER3..=0xB1F => Some(self.mhpmcounter[(a - MHPMCOUNTER3 + 3) as usize]),
            SCOUNTOVF => {
                let mut overflow = 0u64;
                for (i, event) in self.mhpmevent.iter().enumerate().skip(3) {
                    if event & (1 << 63) != 0 {
                        overflow |= 1 << i;
                    }
                }
                Some(overflow)
            }
            MVENDORID => Some(self.mvendorid),
            MARCHID => Some(self.marchid),
            MIMPID => Some(self.mimpid),
            MHARTID => Some(self.mhartid),
            PMPCFG0 => Some(self.pmpcfg[0]),
            PMPCFG2 => Some(self.pmpcfg[1]),
            // On RV64 the odd-numbered pmpcfg registers do not exist.
            PMPCFG1 | PMPCFG3 => None,
            a @ 0x3B0..=0x3BF => Some(self.pmpaddr[(a - PMPADDR0) as usize] & pmp::PMPADDR_MASK),
            MSECCFG => Some(self.mseccfg),
            MSECCFGH => Some(self.mseccfg >> 32),
            SSTATUS => Some(self.mstatus & SSTATUS_MASK),
            SIE => Some(self.mie & SIE_MASK),
            STVEC => Some(self.stvec),
            SCOUNTEREN => Some(self.scounteren),
            SSCRATCH => Some(self.sscratch),
            SEPC => Some(self.sepc),
            SCAUSE => Some(self.scause),
            STVAL => Some(self.stval),
            SIP => Some(self.mip & SIE_MASK),
            SATP => Some(self.satp),
            STIMECMP => Some(self.stimecmp),
            // Unsupported AIA CSRs must trap (illegal instruction) so
            // OpenSBI does not detect SMAIA/SSAIA.
            MTOPI | STOPI => None,
            CYCLE => Some(self.mcycle),
            TIME => Some(insn_count),
            INSTRET => Some(self.minstret),
            _ => Some(0),
        }
    }

    pub fn write(&mut self, addr: u16, val: u64) {
        match addr {
            FFLAGS => self.fcsr = (self.fcsr & !0x1F) | (val as u32 & 0x1F),
            FRM => self.fcsr = (self.fcsr & !(0x7 << 5)) | ((val as u32 & 0x7) << 5),
            FCSR => self.fcsr = val as u32 & 0xFF,
            MSTATUS => self.mstatus = (self.mstatus & !MSTATUS_WRITABLE) | (val & MSTATUS_WRITABLE),
            MEDELEG => self.medeleg = val,
            MIDELEG => self.mideleg = val,
            MIE => self.mie = val & MIE_ALL,
            MTVEC => self.mtvec = val,
            MCOUNTEREN => self.mcounteren = val & 0x7,
            MSCRATCH => self.mscratch = val,
            MEPC => self.mepc = val & !1,
            MCAUSE => self.mcause = val,
            MTVAL => self.mtval = val,
            MIP => self.mip = (self.mip & !MIP_WRITABLE) | (val & MIP_WRITABLE),
            MCOUNTINHIBIT => self.mcountinhibit = val & 0xFFFF_FFFF,
            MCYCLE => self.mcycle = val,
            MINSTRET => self.minstret = val,
            a @ MHPMEVENT3..=0x33F => self.mhpmevent[(a - MHPMEVENT3 + 3) as usize] = val,
            a @ MHPMCOUNTER3..=0xB1F => self.mhpmcounter[(a - MHPMCOUNTER3 + 3) as usize] = val,
            PMPCFG0 => {
                for i in 0..8 {
                    let byte = (val >> (i * 8)) as u8;
                    pmp::write_cfg(self.mseccfg, &mut self.pmpcfg, i, byte);
                }
            }
            PMPCFG2 => {
                for i in 0..8 {
                    let byte = (val >> (i * 8)) as u8;
                    pmp::write_cfg(self.mseccfg, &mut self.pmpcfg, i + 8, byte);
                }
            }
            a @ 0x3B0..=0x3BF => {
                let index = (a - PMPADDR0) as usize;
                pmp::write_addr(self.mseccfg, &self.pmpcfg, &mut self.pmpaddr, index, val);
            }
            MSECCFG => {
                pmp::write_mseccfg(&mut self.mseccfg, &self.pmpcfg, val);
            }
            MSECCFGH => {}
            PMPCFG1 | PMPCFG3 => {}
            SSTATUS => self.mstatus = (self.mstatus & !SSTATUS_WRITABLE) | (val & SSTATUS_WRITABLE),
            SIE => self.mie = (self.mie & !SIE_MASK) | (val & SIE_MASK),
            STVEC => self.stvec = val,
            SCOUNTEREN => self.scounteren = val & 0x7,
            SSCRATCH => self.sscratch = val,
            SEPC => self.sepc = val & !1,
            SCAUSE => self.scause = val,
            STVAL => self.stval = val,
            SIP => self.mip = (self.mip & !(MIP_SSIP | MIP_LCOFI)) | (val & (MIP_SSIP | MIP_LCOFI)),
            SATP => {
                let mode = (val >> 60) & 0xF;
                if mode == 0 || mode == 8 {
                    self.satp = val;
                }
            }
            STIMECMP => {
                self.stimecmp = val;
                self.sstc_active = true;
            }
            MISA | MVENDORID | MARCHID | MIMPID | MHARTID | SCOUNTOVF => {}
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.fcsr = 0;
        self.mstatus = MSTATUS_UXL_64 | MSTATUS_SXL_64;
        self.mie = 0;
        self.mip = 0;
        self.mtvec = 0;
        self.mscratch = 0;
        self.mepc = 0;
        self.mcause = 0;
        self.mtval = 0;
        self.medeleg = 0;
        self.mideleg = 0;
        self.mcounteren = 0;
        self.stvec = 0;
        self.sscratch = 0;
        self.sepc = 0;
        self.scause = 0;
        self.stval = 0;
        self.satp = 0;
        self.stimecmp = u64::MAX;
        self.sstc_active = false;
        self.scounteren = 0;
        self.mcycle = 0;
        self.minstret = 0;
        self.mcountinhibit = 0;
        self.mhpmevent = [0; 32];
        self.mhpmcounter = [0; 32];
        self.pmpcfg = [0; 2];
        self.pmpaddr = [0; 16];
        self.mseccfg = 0;
    }

    /// Advance cycle/instruction/HPM counters by `steps` retired instructions.
    pub fn advance_counters(&mut self, steps: u64) {
        self.advance_counters_split(steps, steps);
    }

    /// Like `advance_counters`, but `minstret` advances by `minstret_steps`.
    /// A write to `minstret` suppresses that instruction's own increment.
    pub fn advance_counters_split(&mut self, steps: u64, minstret_steps: u64) {
        if steps == 0 && minstret_steps == 0 {
            return;
        }
        if self.mcountinhibit & 1 == 0 {
            self.mcycle = self.mcycle.wrapping_add(steps);
        }
        if self.mcountinhibit & 4 == 0 {
            self.minstret = self.minstret.wrapping_add(minstret_steps);
        }
        for i in 3..32 {
            if self.mcountinhibit & (1 << i) != 0 {
                continue;
            }
            let old = self.mhpmcounter[i];
            let new = old.wrapping_add(steps);
            if new < old && self.mhpmevent[i] & (1 << 63) == 0 {
                self.mhpmevent[i] |= 1 << 63;
                self.mip |= MIP_LCOFI;
            }
            self.mhpmcounter[i] = new;
        }
    }

    pub fn pending_interrupt(&self, priv_mode: u8) -> Option<u64> {
        let pending = self.mip & self.mie;
        if pending == 0 {
            return None;
        }

        let m_pending = pending & !self.mideleg;
        let s_pending = pending & self.mideleg;

        let m_global = priv_mode < PRV_M || (self.mstatus & MSTATUS_MIE != 0);
        let s_global = priv_mode < PRV_S || (priv_mode == PRV_S && self.mstatus & MSTATUS_SIE != 0);

        if m_global && m_pending != 0 {
            if m_pending & MIP_MEIP != 0 {
                return Some(11);
            }
            if m_pending & MIP_MSIP != 0 {
                return Some(3);
            }
            if m_pending & MIP_MTIP != 0 {
                return Some(7);
            }
            // Undelegated supervisor-level sources are taken in M-mode too.
            if m_pending & MIP_SEIP != 0 {
                return Some(9);
            }
            if m_pending & MIP_SSIP != 0 {
                return Some(1);
            }
            if m_pending & MIP_STIP != 0 {
                return Some(5);
            }
            if m_pending & MIP_LCOFI != 0 {
                return Some(13);
            }
        }
        if s_global && s_pending != 0 {
            if s_pending & MIP_SEIP != 0 {
                return Some(9);
            }
            if s_pending & MIP_SSIP != 0 {
                return Some(1);
            }
            if s_pending & MIP_STIP != 0 {
                return Some(5);
            }
            if s_pending & MIP_LCOFI != 0 {
                return Some(13);
            }
        }
        None
    }

    pub fn is_delegated(&self, cause: u64) -> bool {
        let is_int = cause & MCAUSE_INTERRUPT != 0;
        let num = cause & !MCAUSE_INTERRUPT;
        if is_int {
            (self.mideleg >> num) & 1 != 0
        } else {
            (self.medeleg >> num) & 1 != 0
        }
    }

    pub fn trap_enter_m(&mut self, pc: u64, cause: u64, tval: u64, prev_mode: u8) -> u64 {
        self.mepc = pc;
        self.mcause = cause;
        self.mtval = tval;
        let mie_bit = self.mstatus & MSTATUS_MIE;
        self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK))
            | (if mie_bit != 0 { MSTATUS_MPIE } else { 0 })
            | ((prev_mode as u64) << 11);
        let mode = self.mtvec & 0x3;
        let base = self.mtvec & !0x3;
        if mode == 1 && cause & MCAUSE_INTERRUPT != 0 {
            base + (cause & !MCAUSE_INTERRUPT) * 4
        } else {
            base
        }
    }

    pub fn trap_enter_s(&mut self, pc: u64, cause: u64, tval: u64, prev_mode: u8) -> u64 {
        self.sepc = pc;
        self.scause = cause;
        self.stval = tval;
        let sie_bit = self.mstatus & MSTATUS_SIE;
        self.mstatus = (self.mstatus & !(MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP))
            | (if sie_bit != 0 { MSTATUS_SPIE } else { 0 })
            | (if prev_mode >= PRV_S { MSTATUS_SPP } else { 0 });
        let mode = self.stvec & 0x3;
        let base = self.stvec & !0x3;
        if mode == 1 && cause & MCAUSE_INTERRUPT != 0 {
            base + (cause & !MCAUSE_INTERRUPT) * 4
        } else {
            base
        }
    }

    pub fn mret(&mut self) -> (u64, u8) {
        let mpie = self.mstatus & MSTATUS_MPIE;
        let mpp = ((self.mstatus & MSTATUS_MPP_MASK) >> 11) as u8;
        self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK))
            | (if mpie != 0 { MSTATUS_MIE } else { 0 })
            | MSTATUS_MPIE;
        (self.mepc, mpp)
    }

    pub fn sret(&mut self) -> (u64, u8) {
        let spie = self.mstatus & MSTATUS_SPIE;
        let spp = if self.mstatus & MSTATUS_SPP != 0 {
            PRV_S
        } else {
            PRV_U
        };
        self.mstatus = (self.mstatus & !(MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP))
            | (if spie != 0 { MSTATUS_SIE } else { 0 })
            | MSTATUS_SPIE;
        (self.sepc, spp)
    }

    pub fn state_size() -> usize {
        106 * 8
    }

    pub fn save(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::state_size());
        for &v in &[
            self.mstatus,
            self.mie,
            self.mip,
            self.mtvec,
            self.mscratch,
            self.mepc,
            self.mcause,
            self.mtval,
            self.medeleg,
            self.mideleg,
            self.stvec,
            self.sscratch,
            self.sepc,
            self.scause,
            self.stval,
            self.satp,
            self.mhartid,
            self.stimecmp,
            self.sstc_active as u64,
            self.fcsr as u64,
            self.mcycle,
            self.minstret,
            self.mcountinhibit,
        ] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for value in self.mhpmevent.iter().chain(self.mhpmcounter.iter()) {
            buf.extend_from_slice(&value.to_le_bytes());
        }
        for &v in &[self.mseccfg, self.pmpcfg[0], self.pmpcfg[1]] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for value in &self.pmpaddr {
            buf.extend_from_slice(&value.to_le_bytes());
        }
        buf
    }

    pub fn load(&mut self, data: &[u8]) {
        // Snapshot layout history:
        //   8 fields  (64 B)  — original M-mode-only format
        //   17 fields (136 B) — M+S CSRs, before Sstc
        //   19 fields (152 B) — adds stimecmp + sstc_active
        if data.len() < 8 * 8 {
            return;
        }
        let r = |off: usize| -> u64 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&data[off..off + 8]);
            u64::from_le_bytes(buf)
        };

        self.mstatus = r(0);
        self.mie = r(8);
        self.mip = r(16);
        self.mtvec = r(24);
        self.mscratch = r(32);
        self.mepc = r(40);
        self.mcause = r(48);
        self.mtval = r(56);

        if data.len() >= 17 * 8 {
            self.medeleg = r(64);
            self.mideleg = r(72);
            self.stvec = r(80);
            self.sscratch = r(88);
            self.sepc = r(96);
            self.scause = r(104);
            self.stval = r(112);
            self.satp = r(120);
            self.mhartid = r(128);
        } else {
            return;
        }

        if data.len() >= 20 * 8 {
            self.stimecmp = r(136);
            self.sstc_active = r(144) != 0;
            self.fcsr = r(152) as u32;
        } else {
            // Pre-Sstc 17-field snapshot: timer state was not recorded.
            self.stimecmp = u64::MAX;
            self.sstc_active = false;
            self.fcsr = 0;
        }

        if data.len() >= 87 * 8 {
            self.mcycle = r(160);
            self.minstret = r(168);
            self.mcountinhibit = r(176);
            for i in 0..32 {
                self.mhpmevent[i] = r(184 + i * 8);
                self.mhpmcounter[i] = r(440 + i * 8);
            }
        } else {
            self.mcycle = 0;
            self.minstret = 0;
            self.mcountinhibit = 0;
            self.mhpmevent = [0; 32];
            self.mhpmcounter = [0; 32];
        }

        if data.len() >= Self::state_size() {
            self.mseccfg = r(696);
            self.pmpcfg[0] = r(704);
            self.pmpcfg[1] = r(712);
            for (i, addr) in self.pmpaddr.iter_mut().enumerate() {
                *addr = r(720 + i * 8);
            }
        } else {
            self.mseccfg = 0;
            self.pmpcfg = [0; 2];
            self.pmpaddr = [0; 16];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_misa() {
        let csr = CsrFile::new(0);
        let misa = csr.read(MISA, 0).unwrap();
        assert_eq!(misa >> 62, 2);
        assert!(misa & (1 << 8) != 0); // I
        assert!(misa & (1 << 12) != 0); // M
        assert!(misa & (1 << 0) != 0); // A
        assert!(misa & (1 << 2) != 0); // C
        assert!(misa & (1 << 18) != 0); // S
        assert!(misa & (1 << 20) != 0); // U
    }

    #[test]
    fn test_misa_without_compressed() {
        let csr = CsrFile::new_with_type(0, RiscvCpuType::RhetSimRv64Ima);
        let misa = csr.read(MISA, 0).unwrap();
        assert_eq!(misa & (1 << 2), 0); // C
    }

    #[test]
    fn test_fcsr_views() {
        let mut csr = CsrFile::new(0);
        csr.write(FCSR, 0x5A);
        assert_eq!(csr.read(FCSR, 0), Some(0x5A));
        assert_eq!(csr.read(FFLAGS, 0), Some(0x1A));
        assert_eq!(csr.read(FRM, 0), Some(2));

        csr.write(FFLAGS, 0x05);
        assert_eq!(csr.read(FCSR, 0), Some(0x45));
        csr.write(FRM, 0x03);
        assert_eq!(csr.read(FCSR, 0), Some(0x65));
    }

    #[test]
    fn test_sscofpmf_counters() {
        let mut csr = CsrFile::new(0);
        csr.advance_counters(5);
        assert_eq!(csr.read(MCYCLE, 0), Some(5));
        assert_eq!(csr.read(MINSTRET, 0), Some(5));
        assert_eq!(csr.read(MHPMCOUNTER3, 0), Some(5));

        csr.write(MCOUNTINHIBIT, 1);
        csr.advance_counters(1);
        assert_eq!(csr.read(MCYCLE, 0), Some(5));
        assert_eq!(csr.read(MINSTRET, 0), Some(6));

        csr.write(MCOUNTINHIBIT, 0);
        csr.mhpmcounter[3] = u64::MAX;
        csr.advance_counters(1);
        assert_ne!(csr.mhpmevent[3] & (1 << 63), 0);
        assert_ne!(csr.mip & MIP_LCOFI, 0);
        assert_ne!(csr.read(SCOUNTOVF, 0).unwrap() & (1 << 3), 0);
    }

    #[test]
    fn test_pending_interrupt() {
        let mut csr = CsrFile::new(0);
        assert!(csr.pending_interrupt(PRV_M).is_none());

        csr.mip = MIP_MTIP;
        csr.mie = MIP_MTIP;
        assert!(csr.pending_interrupt(PRV_M).is_none());

        csr.mstatus = MSTATUS_MIE;
        assert_eq!(csr.pending_interrupt(PRV_M), Some(7));

        csr.mip |= MIP_MEIP;
        csr.mie |= MIP_MEIP;
        assert_eq!(csr.pending_interrupt(PRV_M), Some(11));
    }

    #[test]
    fn test_delegated_interrupt() {
        let mut csr = CsrFile::new(0);
        csr.mip = MIP_STIP;
        csr.mie = MIP_STIP;
        csr.mideleg = MIP_STIP;
        csr.mstatus = MSTATUS_SIE;

        assert_eq!(csr.pending_interrupt(PRV_S), Some(5));
        assert!(csr.pending_interrupt(PRV_M).is_none());
    }

    #[test]
    fn test_sstatus_view() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE | MSTATUS_SIE | MSTATUS_SPP;

        let sstatus = csr.read(SSTATUS, 0).unwrap();
        assert!(sstatus & MSTATUS_SIE != 0);
        assert!(sstatus & MSTATUS_SPP != 0);
        assert!(sstatus & MSTATUS_MIE == 0); // MIE not visible through sstatus
    }

    #[test]
    fn test_sie_sip_view() {
        let mut csr = CsrFile::new(0);
        csr.mie = MIP_MTIP | MIP_STIP;
        assert_eq!(csr.read(SIE, 0).unwrap(), MIP_STIP);

        csr.mip = MIP_MSIP | MIP_SSIP;
        assert_eq!(csr.read(SIP, 0).unwrap(), MIP_SSIP);
    }

    #[test]
    fn test_trap_enter_mret() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x8000_1000;

        let target = csr.trap_enter_m(0x8000_0100, MCAUSE_INTERRUPT | 7, 0, PRV_M);
        assert_eq!(target, 0x8000_1000);
        assert_eq!(csr.mepc, 0x8000_0100);
        assert_eq!(csr.mcause, MCAUSE_INTERRUPT | 7);
        assert!(csr.mstatus & MSTATUS_MIE == 0);
        assert!(csr.mstatus & MSTATUS_MPIE != 0);
        assert_eq!((csr.mstatus & MSTATUS_MPP_MASK) >> 11, PRV_M as u64);

        let (ret_pc, ret_mode) = csr.mret();
        assert_eq!(ret_pc, 0x8000_0100);
        assert_eq!(ret_mode, PRV_M);
        assert!(csr.mstatus & MSTATUS_MIE != 0);
    }

    #[test]
    fn test_trap_enter_sret() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_SIE;
        csr.stvec = 0x8000_2000;

        let target = csr.trap_enter_s(0x8000_0200, 13, 0xDEAD, PRV_S);
        assert_eq!(target, 0x8000_2000);
        assert_eq!(csr.sepc, 0x8000_0200);
        assert_eq!(csr.scause, 13);
        assert_eq!(csr.stval, 0xDEAD);
        assert!(csr.mstatus & MSTATUS_SIE == 0);
        assert!(csr.mstatus & MSTATUS_SPIE != 0);
        assert!(csr.mstatus & MSTATUS_SPP != 0);

        let (ret_pc, ret_mode) = csr.sret();
        assert_eq!(ret_pc, 0x8000_0200);
        assert_eq!(ret_mode, PRV_S);
        assert!(csr.mstatus & MSTATUS_SIE != 0);
    }

    #[test]
    fn test_vectored_mtvec() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x8000_1001;

        let target = csr.trap_enter_m(0x8000_0100, MCAUSE_INTERRUPT | 7, 0, PRV_M);
        assert_eq!(target, 0x8000_1000 + 7 * 4);
    }

    #[test]
    fn test_delegation() {
        let mut csr = CsrFile::new(0);
        csr.medeleg = (1 << 13) | (1 << 15);
        assert!(csr.is_delegated(13));
        assert!(csr.is_delegated(15));
        assert!(!csr.is_delegated(11));

        csr.mideleg = MIP_STIP;
        assert!(csr.is_delegated(MCAUSE_INTERRUPT | 5));
        assert!(!csr.is_delegated(MCAUSE_INTERRUPT | 7));
    }

    #[test]
    fn test_mret_to_s_mode() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE | MSTATUS_MPIE | (PRV_S as u64) << 11;
        csr.mepc = 0x8000_2000;

        let (pc, mode) = csr.mret();
        assert_eq!(pc, 0x8000_2000);
        assert_eq!(mode, PRV_S);
    }

    #[test]
    fn test_stimecmp() {
        let mut csr = CsrFile::new(0);
        assert_eq!(csr.read(STIMECMP, 0).unwrap(), u64::MAX);
        assert!(!csr.sstc_active);

        csr.write(STIMECMP, 12345);
        assert_eq!(csr.read(STIMECMP, 0).unwrap(), 12345);
        assert!(csr.sstc_active);

        csr.reset();
        assert_eq!(csr.read(STIMECMP, 0).unwrap(), u64::MAX);
        assert!(!csr.sstc_active);
    }

    #[test]
    fn test_mip_software_injection_bits_writable() {
        let mut csr = CsrFile::new(0);
        csr.write(MIP, MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_MSIP);
        assert_eq!(
            csr.mip & (MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_MSIP),
            MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_MSIP
        );
        // Hardware-driven bits stay read-only.
        csr.write(MIP, MIP_MTIP | MIP_MEIP);
        assert_eq!(csr.mip & (MIP_MTIP | MIP_MEIP), 0);
    }

    #[test]
    fn test_save_load() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x1234;
        csr.mepc = 0x5678;
        csr.stvec = 0x9ABC;
        csr.satp = 0xDEF0;
        csr.stimecmp = 0x1234_5678_9ABC_DEF0;
        csr.sstc_active = true;
        let data = csr.save();

        let mut csr2 = CsrFile::new(0);
        csr2.load(&data);
        assert_eq!(csr2.mstatus, MSTATUS_MIE);
        assert_eq!(csr2.mtvec, 0x1234);
        assert_eq!(csr2.mepc, 0x5678);
        assert_eq!(csr2.stvec, 0x9ABC);
        assert_eq!(csr2.satp, 0xDEF0);
        assert_eq!(csr2.stimecmp, 0x1234_5678_9ABC_DEF0);
        assert!(csr2.sstc_active);
    }

    #[test]
    fn test_load_pre_sstc_snapshot() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.stvec = 0x9ABC;
        csr.satp = 0xDEF0;
        csr.stimecmp = 42;
        csr.sstc_active = true;

        let data = csr.save();
        let legacy = &data[..17 * 8];

        let mut csr2 = CsrFile::new(0);
        csr2.load(legacy);
        assert_eq!(csr2.mstatus, MSTATUS_MIE);
        assert_eq!(csr2.stvec, 0x9ABC);
        assert_eq!(csr2.satp, 0xDEF0);
        assert_eq!(csr2.stimecmp, u64::MAX);
        assert!(!csr2.sstc_active);
    }
}
