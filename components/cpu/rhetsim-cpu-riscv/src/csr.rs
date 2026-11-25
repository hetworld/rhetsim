//! RISC-V M-mode CSR file

pub const MSTATUS: u16   = 0x300;
pub const MISA: u16      = 0x301;
pub const MIE: u16       = 0x304;
pub const MTVEC: u16     = 0x305;
pub const MSCRATCH: u16  = 0x340;
pub const MEPC: u16      = 0x341;
pub const MCAUSE: u16    = 0x342;
pub const MTVAL: u16     = 0x343;
pub const MIP: u16       = 0x344;
pub const MHARTID: u16   = 0xF14;
pub const MCYCLE: u16    = 0xB00;
pub const MINSTRET: u16  = 0xB02;

// mstatus bit positions
pub const MSTATUS_MIE: u64  = 1 << 3;
pub const MSTATUS_MPIE: u64 = 1 << 7;
pub const MSTATUS_MPP_MASK: u64 = 3 << 11;
pub const MSTATUS_MPP_M: u64 = 3 << 11;

// mie/mip bit positions
pub const MIP_MSIP: u64 = 1 << 3;   // machine software interrupt
pub const MIP_MTIP: u64 = 1 << 7;   // machine timer interrupt
pub const MIP_MEIP: u64 = 1 << 11;  // machine external interrupt

// mcause interrupt bit
pub const MCAUSE_INTERRUPT: u64 = 1u64 << 63;

// misa: RV64IMAG
fn misa_rv64() -> u64 {
    let mxl = 2u64 << 62; // MXL=2 → 64-bit
    let ext_i = 1 << 8;
    let ext_m = 1 << 12;
    let ext_a = 1 << 0;
    mxl | ext_i | ext_m | ext_a
}

pub struct CsrFile {
    pub mstatus: u64,
    pub mie: u64,
    pub mip: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub mhartid: u64,
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
        }
    }

    pub fn read(&self, addr: u16, insn_count: u64) -> Option<u64> {
        match addr {
            MSTATUS   => Some(self.mstatus),
            MISA      => Some(misa_rv64()),
            MIE       => Some(self.mie),
            MTVEC     => Some(self.mtvec),
            MSCRATCH  => Some(self.mscratch),
            MEPC      => Some(self.mepc),
            MCAUSE    => Some(self.mcause),
            MTVAL     => Some(self.mtval),
            MIP       => Some(self.mip),
            MHARTID   => Some(self.mhartid),
            MCYCLE    => Some(insn_count),
            MINSTRET  => Some(insn_count),
            _ => Some(0),
        }
    }

    pub fn write(&mut self, addr: u16, val: u64) {
        match addr {
            MSTATUS   => {
                let writable = MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK;
                self.mstatus = (self.mstatus & !writable) | (val & writable);
            }
            MIE       => self.mie = val & (MIP_MSIP | MIP_MTIP | MIP_MEIP),
            MTVEC     => self.mtvec = val,
            MSCRATCH  => self.mscratch = val,
            MEPC      => self.mepc = val & !1,
            MCAUSE    => self.mcause = val,
            MTVAL     => self.mtval = val,
            MIP       => {
                // only MSIP is writable via CSR; MTIP and MEIP are read-only
                self.mip = (self.mip & !(MIP_MSIP)) | (val & MIP_MSIP);
            }
            MISA | MHARTID | MCYCLE | MINSTRET => {} // read-only
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
    }

    pub fn pending_interrupt(&self) -> Option<u64> {
        if self.mstatus & MSTATUS_MIE == 0 {
            return None;
        }
        let pending = self.mip & self.mie;
        if pending == 0 {
            return None;
        }
        // priority: MEI > MSI > MTI
        if pending & MIP_MEIP != 0 { return Some(11); }
        if pending & MIP_MSIP != 0 { return Some(3); }
        if pending & MIP_MTIP != 0 { return Some(7); }
        None
    }

    pub fn trap_enter(&mut self, pc: u64, cause: u64, tval: u64) -> u64 {
        self.mepc = pc;
        self.mcause = cause;
        self.mtval = tval;
        // save MIE to MPIE, clear MIE, set MPP to M-mode
        let mie_bit = self.mstatus & MSTATUS_MIE;
        self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK))
            | (if mie_bit != 0 { MSTATUS_MPIE } else { 0 })
            | MSTATUS_MPP_M;
        // compute target PC from mtvec
        let mode = self.mtvec & 0x3;
        let base = self.mtvec & !0x3;
        if mode == 1 && cause & MCAUSE_INTERRUPT != 0 {
            base + (cause & !MCAUSE_INTERRUPT) * 4
        } else {
            base
        }
    }

    pub fn mret(&mut self) -> u64 {
        // restore MIE from MPIE, set MPIE=1, set MPP to U (but we only do M-mode)
        let mpie = self.mstatus & MSTATUS_MPIE;
        self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK))
            | (if mpie != 0 { MSTATUS_MIE } else { 0 })
            | MSTATUS_MPIE;
        self.mepc
    }

    pub fn state_size() -> usize { 8 * 8 }

    pub fn save(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::state_size());
        for &v in &[self.mstatus, self.mie, self.mip, self.mtvec,
                     self.mscratch, self.mepc, self.mcause, self.mtval] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf
    }

    pub fn load(&mut self, data: &[u8]) {
        let mut read_u64 = |off: usize| -> u64 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&data[off..off + 8]);
            u64::from_le_bytes(buf)
        };
        self.mstatus  = read_u64(0);
        self.mie      = read_u64(8);
        self.mip      = read_u64(16);
        self.mtvec    = read_u64(24);
        self.mscratch = read_u64(32);
        self.mepc     = read_u64(40);
        self.mcause   = read_u64(48);
        self.mtval    = read_u64(56);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_misa() {
        let csr = CsrFile::new(0);
        let misa = csr.read(MISA, 0).unwrap();
        assert_eq!(misa >> 62, 2); // RV64
        assert!(misa & (1 << 8) != 0); // I
        assert!(misa & (1 << 12) != 0); // M
        assert!(misa & (1 << 0) != 0); // A
    }

    #[test]
    fn test_pending_interrupt() {
        let mut csr = CsrFile::new(0);
        assert!(csr.pending_interrupt().is_none());

        csr.mip = MIP_MTIP;
        csr.mie = MIP_MTIP;
        assert!(csr.pending_interrupt().is_none()); // MIE disabled

        csr.mstatus = MSTATUS_MIE;
        assert_eq!(csr.pending_interrupt(), Some(7)); // timer

        csr.mip |= MIP_MEIP;
        csr.mie |= MIP_MEIP;
        assert_eq!(csr.pending_interrupt(), Some(11)); // external has priority
    }

    #[test]
    fn test_trap_enter_mret() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x8000_1000;

        let target = csr.trap_enter(0x8000_0100, MCAUSE_INTERRUPT | 7, 0);
        assert_eq!(target, 0x8000_1000); // direct mode
        assert_eq!(csr.mepc, 0x8000_0100);
        assert_eq!(csr.mcause, MCAUSE_INTERRUPT | 7);
        assert!(csr.mstatus & MSTATUS_MIE == 0); // MIE cleared
        assert!(csr.mstatus & MSTATUS_MPIE != 0); // MPIE saved

        let ret_pc = csr.mret();
        assert_eq!(ret_pc, 0x8000_0100);
        assert!(csr.mstatus & MSTATUS_MIE != 0); // MIE restored
    }

    #[test]
    fn test_vectored_mtvec() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x8000_1001; // vectored mode (bit 0 = 1)

        let target = csr.trap_enter(0x8000_0100, MCAUSE_INTERRUPT | 7, 0);
        assert_eq!(target, 0x8000_1000 + 7 * 4); // base + cause * 4
    }

    #[test]
    fn test_save_load() {
        let mut csr = CsrFile::new(0);
        csr.mstatus = MSTATUS_MIE;
        csr.mtvec = 0x1234;
        csr.mepc = 0x5678;
        let data = csr.save();

        let mut csr2 = CsrFile::new(0);
        csr2.load(&data);
        assert_eq!(csr2.mstatus, MSTATUS_MIE);
        assert_eq!(csr2.mtvec, 0x1234);
        assert_eq!(csr2.mepc, 0x5678);
    }
}
