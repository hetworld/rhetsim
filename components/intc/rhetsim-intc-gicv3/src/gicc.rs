//! GICv3 CPU Interface
//!
//! In GICv3, the CPU interface is accessed via system registers (ICC_*),
//! not memory-mapped. This struct models the per-CPU interface state:
//!   - PMR (Priority Mask Register): only interrupts with priority < PMR are signaled
//!   - BPR (Binary Point Register): priority grouping
//!   - IAR (Interrupt Acknowledge Register): returns INTID on read
//!   - EOIR (End of Interrupt Register): completes interrupt on write
//!   - Group enable (IGRPEN1)

pub struct Gicc {
    cpu_id: u8,
    pmr: u8,
    bpr: u8,
    group1_enabled: bool,
}

impl Gicc {
    pub fn new(cpu_id: u8) -> Self {
        Self {
            cpu_id,
            pmr: 0,
            bpr: 0,
            group1_enabled: false,
        }
    }

    pub fn cpu_id(&self) -> u8 {
        self.cpu_id
    }

    pub fn pmr(&self) -> u8 {
        self.pmr
    }
    pub fn set_pmr(&mut self, val: u8) {
        self.pmr = val;
    }

    pub fn bpr(&self) -> u8 {
        self.bpr
    }
    pub fn set_bpr(&mut self, val: u8) {
        self.bpr = val;
    }

    pub fn group1_enabled(&self) -> bool {
        self.group1_enabled
    }
    pub fn set_group1_enabled(&mut self, en: bool) {
        self.group1_enabled = en;
    }

    pub fn reset(&mut self) {
        self.pmr = 0;
        self.bpr = 0;
        self.group1_enabled = false;
    }
}
