//! RISC-V CSR file — M-mode and S-mode

// ── CSR addresses ─────────────────────────────────────────────

// S-mode
pub const SSTATUS: u16   = 0x100;
pub const SIE: u16       = 0x104;
pub const STVEC: u16     = 0x105;
pub const SSCRATCH: u16  = 0x140;
pub const SEPC: u16      = 0x141;
pub const SCAUSE: u16    = 0x142;
pub const STVAL: u16     = 0x143;
pub const SIP: u16       = 0x144;
pub const SATP: u16      = 0x180;
/// Sstc supervisor timer compare (RV64: 64-bit CSR).
pub const STIMECMP: u16  = 0x14D;

// M-mode
pub const MSTATUS: u16   = 0x300;
pub const MISA: u16      = 0x301;
pub const MEDELEG: u16   = 0x302;
pub const MIDELEG: u16   = 0x303;
pub const MIE: u16       = 0x304;
pub const MTVEC: u16     = 0x305;
pub const MCOUNTEREN: u16 = 0x306;
pub const MSCRATCH: u16  = 0x340;
pub const MEPC: u16      = 0x341;
pub const MCAUSE: u16    = 0x342;
pub const MTVAL: u16     = 0x343;
pub const MIP: u16       = 0x344;

// PMP
pub const PMPCFG0: u16   = 0x3A0;
pub const PMPCFG2: u16   = 0x3A2;
pub const PMPADDR0: u16  = 0x3B0;

// Counters (M-mode)
pub const MCYCLE: u16    = 0xB00;
pub const MINSTRET: u16  = 0xB02;

// Counters (S/U-mode readable)
pub const CYCLE: u16     = 0xC00;
pub const TIME: u16      = 0xC01;
pub const INSTRET: u16   = 0xC02;

// Machine info (read-only)
pub const MVENDORID: u16 = 0xF11;
pub const MARCHID: u16   = 0xF12;
pub const MIMPID: u16    = 0xF13;
pub const MHARTID: u16   = 0xF14;

// S-mode counter enable
pub const SCOUNTEREN: u16 = 0x106;

// ── mstatus bit positions ─────────────────────────────────────

pub const MSTATUS_SIE: u64  = 1 << 1;
pub const MSTATUS_MIE: u64  = 1 << 3;
pub const MSTATUS_SPIE: u64 = 1 << 5;
pub const MSTATUS_MPIE: u64 = 1 << 7;
pub const MSTATUS_SPP: u64  = 1 << 8;
pub const MSTATUS_MPP_MASK: u64 = 3 << 11;
pub const MSTATUS_MPP_M: u64 = 3 << 11;
pub const MSTATUS_MPRV: u64 = 1 << 17;
pub const MSTATUS_SUM: u64  = 1 << 18;
pub const MSTATUS_MXR: u64  = 1 << 19;
pub const MSTATUS_TVM: u64  = 1 << 20;
pub const MSTATUS_TW: u64   = 1 << 21;
pub const MSTATUS_TSR: u64  = 1 << 22;

const MSTATUS_WRITABLE: u64 = MSTATUS_SIE | MSTATUS_MIE | MSTATUS_SPIE | MSTATUS_MPIE
    | MSTATUS_SPP | MSTATUS_MPP_MASK | MSTATUS_MPRV | MSTATUS_SUM | MSTATUS_MXR
    | MSTATUS_TVM | MSTATUS_TW | MSTATUS_TSR;

const SSTATUS_MASK: u64 = MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP
    | (3 << 13) | (3 << 15) | MSTATUS_SUM | MSTATUS_MXR | (1u64 << 63);

const SSTATUS_WRITABLE: u64 = MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP
    | MSTATUS_SUM | MSTATUS_MXR;

// ── mie/mip bit positions ─────────────────────────────────────

pub const MIP_SSIP: u64 = 1 << 1;
pub const MIP_MSIP: u64 = 1 << 3;
pub const MIP_STIP: u64 = 1 << 5;
pub const MIP_MTIP: u64 = 1 << 7;
pub const MIP_SEIP: u64 = 1 << 9;
pub const MIP_MEIP: u64 = 1 << 11;

const SIE_MASK: u64 = MIP_SSIP | MIP_STIP | MIP_SEIP;
const MIE_ALL: u64  = MIP_SSIP | MIP_MSIP | MIP_STIP | MIP_MTIP | MIP_SEIP | MIP_MEIP;

/// M-mode can write software-injected interrupt bits:
/// - MSIP: software interrupt (CLINT/rv-virt)
/// - SSIP/STIP/SEIP: needed by OpenSBI to inject S-mode IPI/timer/external
///
/// MEIP/MTIP are driven by the interrupt controllers (PLIC/CLINT) and stay
/// read-only; the CPU syncs them from IrqLineSource.
const MIP_WRITABLE: u64 = MIP_MSIP | MIP_SSIP | MIP_STIP | MIP_SEIP;

// ── mcause ────────────────────────────────────────────────────

pub const MCAUSE_INTERRUPT: u64 = 1u64 << 63;

// ── Privilege modes ───────────────────────────────────────────

pub const PRV_U: u8 = 0;
pub const PRV_S: u8 = 1;
pub const PRV_M: u8 = 3;

// ── misa: RV64IMASU ───────────────────────────────────────────

fn misa_rv64() -> u64 {
    let mxl = 2u64 << 62;
    let ext_i = 1 << 8;
    let ext_m = 1 << 12;
    let ext_a = 1 << 0;
    let ext_s = 1 << 18;
    let ext_u = 1 << 20;
    mxl | ext_i | ext_m | ext_a | ext_s | ext_u
}

// ── CsrFile ───────────────────────────────────────────────────

pub struct CsrFile {
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
    // PMP (store-only, no enforcement)
    pub pmpcfg: [u64; 2],
    pub pmpaddr: [u64; 16],
}

impl CsrFile {
    pub fn new(hart_id: u64) -> Self {
        Self {
            mstatus: 0,
            mie: 0,
            mip: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mhartid: hart_id,
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
        }
    }

    pub fn read(&self, addr: u16, insn_count: u64) -> Option<u64> {
        match addr {
            MSTATUS   => Some(self.mstatus),
            MISA      => Some(misa_rv64()),
            MEDELEG   => Some(self.medeleg),
            MIDELEG   => Some(self.mideleg),
            MIE       => Some(self.mie),
            MTVEC     => Some(self.mtvec),
            MCOUNTEREN => Some(self.mcounteren),
            MSCRATCH  => Some(self.mscratch),
            MEPC      => Some(self.mepc),
            MCAUSE    => Some(self.mcause),
            MTVAL     => Some(self.mtval),
            MIP       => Some(self.mip),
            MCYCLE    => Some(insn_count),
            MINSTRET  => Some(insn_count),
            MVENDORID => Some(0),
            MARCHID   => Some(0),
            MIMPID    => Some(0),
            MHARTID   => Some(self.mhartid),
            PMPCFG0   => Some(self.pmpcfg[0]),
            PMPCFG2   => Some(self.pmpcfg[1]),
            a @ 0x3B0..=0x3BF => Some(self.pmpaddr[(a - PMPADDR0) as usize]),
            SSTATUS   => Some(self.mstatus & SSTATUS_MASK),
            SIE       => Some(self.mie & SIE_MASK),
            STVEC     => Some(self.stvec),
            SCOUNTEREN => Some(self.scounteren),
            SSCRATCH  => Some(self.sscratch),
            SEPC      => Some(self.sepc),
            SCAUSE    => Some(self.scause),
            STVAL     => Some(self.stval),
            SIP       => Some(self.mip & SIE_MASK),
            SATP      => Some(self.satp),
            STIMECMP  => Some(self.stimecmp),
            CYCLE     => Some(insn_count),
            TIME      => Some(insn_count),
            INSTRET   => Some(insn_count),
            _ => Some(0),
        }
    }

    pub fn write(&mut self, addr: u16, val: u64) {
        match addr {
            MSTATUS   => self.mstatus = (self.mstatus & !MSTATUS_WRITABLE) | (val & MSTATUS_WRITABLE),
            MEDELEG   => self.medeleg = val,
            MIDELEG   => self.mideleg = val,
            MIE       => self.mie = val & MIE_ALL,
            MTVEC     => self.mtvec = val,
            MCOUNTEREN => self.mcounteren = val & 0x7,
            MSCRATCH  => self.mscratch = val,
            MEPC      => self.mepc = val & !1,
            MCAUSE    => self.mcause = val,
            MTVAL     => self.mtval = val,
            MIP       => self.mip = (self.mip & !MIP_WRITABLE) | (val & MIP_WRITABLE),
            PMPCFG0   => self.pmpcfg[0] = val,
            PMPCFG2   => self.pmpcfg[1] = val,
            a @ 0x3B0..=0x3BF => self.pmpaddr[(a - PMPADDR0) as usize] = val,
            SSTATUS   => self.mstatus = (self.mstatus & !SSTATUS_WRITABLE) | (val & SSTATUS_WRITABLE),
            SIE       => self.mie = (self.mie & !SIE_MASK) | (val & SIE_MASK),
            STVEC     => self.stvec = val,
            SCOUNTEREN => self.scounteren = val & 0x7,
            SSCRATCH  => self.sscratch = val,
            SEPC      => self.sepc = val & !1,
            SCAUSE    => self.scause = val,
            STVAL     => self.stval = val,
            SIP       => self.mip = (self.mip & !MIP_SSIP) | (val & MIP_SSIP),
            SATP      => {
                let mode = (val >> 60) & 0xF;
                if mode == 0 || mode == 8 {
                    self.satp = val;
                }
            }
            STIMECMP  => {
                self.stimecmp = val;
                self.sstc_active = true;
            }
            MISA | MVENDORID | MARCHID | MIMPID | MHARTID | MCYCLE | MINSTRET => {}
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.mstatus = 0;
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
        self.pmpcfg = [0; 2];
        self.pmpaddr = [0; 16];
    }

    pub fn pending_interrupt(&self, priv_mode: u8) -> Option<u64> {
        let pending = self.mip & self.mie;
        if pending == 0 { return None; }

        let m_pending = pending & !self.mideleg;
        let s_pending = pending & self.mideleg;

        let m_global = priv_mode < PRV_M || (self.mstatus & MSTATUS_MIE != 0);
        let s_global = priv_mode < PRV_S
            || (priv_mode == PRV_S && self.mstatus & MSTATUS_SIE != 0);

        if m_global && m_pending != 0 {
            if m_pending & MIP_MEIP != 0 { return Some(11); }
            if m_pending & MIP_MSIP != 0 { return Some(3); }
            if m_pending & MIP_MTIP != 0 { return Some(7); }
        }
        if s_global && s_pending != 0 {
            if s_pending & MIP_SEIP != 0 { return Some(9); }
            if s_pending & MIP_SSIP != 0 { return Some(1); }
            if s_pending & MIP_STIP != 0 { return Some(5); }
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
        let spp = if self.mstatus & MSTATUS_SPP != 0 { PRV_S } else { PRV_U };
        self.mstatus = (self.mstatus & !(MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP))
            | (if spie != 0 { MSTATUS_SIE } else { 0 })
            | MSTATUS_SPIE;
        (self.sepc, spp)
    }

    pub fn state_size() -> usize { 19 * 8 }

    pub fn save(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::state_size());
        for &v in &[self.mstatus, self.mie, self.mip, self.mtvec,
                     self.mscratch, self.mepc, self.mcause, self.mtval,
                     self.medeleg, self.mideleg,
                     self.stvec, self.sscratch, self.sepc, self.scause, self.stval,
                     self.satp, self.mhartid, self.stimecmp,
                     self.sstc_active as u64] {
            buf.extend_from_slice(&v.to_le_bytes());
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

        self.mstatus  = r(0);
        self.mie      = r(8);
        self.mip      = r(16);
        self.mtvec    = r(24);
        self.mscratch = r(32);
        self.mepc     = r(40);
        self.mcause   = r(48);
        self.mtval    = r(56);

        if data.len() >= 17 * 8 {
            self.medeleg  = r(64);
            self.mideleg  = r(72);
            self.stvec    = r(80);
            self.sscratch = r(88);
            self.sepc     = r(96);
            self.scause   = r(104);
            self.stval    = r(112);
            self.satp     = r(120);
            self.mhartid  = r(128);
        } else {
            return;
        }

        if data.len() >= Self::state_size() {
            self.stimecmp    = r(136);
            self.sstc_active = r(144) != 0;
        } else {
            // Pre-Sstc 17-field snapshot: timer state was not recorded.
            self.stimecmp    = u64::MAX;
            self.sstc_active = false;
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
        assert!(misa & (1 << 8) != 0);  // I
        assert!(misa & (1 << 12) != 0); // M
        assert!(misa & (1 << 0) != 0);  // A
        assert!(misa & (1 << 18) != 0); // S
        assert!(misa & (1 << 20) != 0); // U
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
        assert_eq!(csr.mip & (MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_MSIP),
                   MIP_SSIP | MIP_STIP | MIP_SEIP | MIP_MSIP);
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
