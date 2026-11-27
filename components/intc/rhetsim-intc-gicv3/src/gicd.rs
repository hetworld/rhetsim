//! GICv3 Distributor (GICD)
//!
//! Memory map (per ARM GICv3 spec, subset):
//!   0x0000  GICD_CTLR        — control (enable groups)
//!   0x0004  GICD_TYPER       — type (IT lines, CPUs)
//!   0x0080  GICD_IGROUPR     — interrupt group (32 bits per word, SPI starting at 32)
//!   0x0100  GICD_ISENABLER   — set-enable (32 bits per word)
//!   0x0180  GICD_ICENABLER   — clear-enable
//!   0x0200  GICD_ISPENDR     — set-pending
//!   0x0280  GICD_ICPENDR     — clear-pending
//!   0x0400  GICD_IPRIORITYR  — priority (1 byte per INTID)
//!   0x0800  GICD_ITARGETSR   — target (1 byte per INTID, GICv2 compat)
//!   0x0C00  GICD_ICFGR       — config (2 bits per INTID)
//!   0x6000  GICD_IROUTER     — affinity routing (8 bytes per SPI, INTID 32+)
//!
//! This distributor handles SPIs (Shared Peripheral Interrupts, INTID 32..1020).
//! SGIs (0..15) and PPIs (16..31) are managed by the Redistributor.

use std::sync::RwLock;

const MAX_SPIS: usize = 988; // INTIDs 32..1019
const MAX_INTIDS: usize = 1020;

pub struct Gicd {
    enabled: bool,
    num_spis: usize,
    enable: RwLock<Vec<u32>>,  // set-enable bits, indexed by word (INTID/32)
    pending: RwLock<Vec<u32>>, // pending bits
    active: RwLock<Vec<u32>>,  // active bits
    priority: RwLock<Vec<u8>>, // per-INTID priority
    target_cpu: RwLock<Vec<u8>>, // affinity target CPU for each SPI
}

impl Gicd {
    pub fn new(num_spis: usize) -> Self {
        assert!(num_spis <= MAX_SPIS);
        let total = 32 + num_spis; // SGI/PPI (0..31) + SPIs
        let words = (total + 31) / 32;
        Self {
            enabled: false,
            num_spis,
            enable: RwLock::new(vec![0u32; words]),
            pending: RwLock::new(vec![0u32; words]),
            active: RwLock::new(vec![0u32; words]),
            priority: RwLock::new(vec![0xFFu8; total]),
            target_cpu: RwLock::new(vec![0u8; num_spis]),
        }
    }

    pub fn num_intids(&self) -> usize {
        32 + self.num_spis
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, en: bool) {
        self.enabled = en;
    }

    pub fn set_spi_enable(&self, intid: usize, en: bool) {
        if intid < 32 || intid >= self.num_intids() {
            return;
        }
        let word = intid / 32;
        let bit = intid % 32;
        let mut v = self.enable.write().unwrap();
        if en {
            v[word] |= 1 << bit;
        } else {
            v[word] &= !(1 << bit);
        }
    }

    pub fn is_spi_enabled(&self, intid: usize) -> bool {
        if intid < 32 || intid >= self.num_intids() {
            return false;
        }
        let word = intid / 32;
        let bit = intid % 32;
        (self.enable.read().unwrap()[word] >> bit) & 1 != 0
    }

    pub fn set_spi_pending(&self, intid: usize, pend: bool) {
        if intid < 32 || intid >= self.num_intids() {
            return;
        }
        let word = intid / 32;
        let bit = intid % 32;
        let mut v = self.pending.write().unwrap();
        if pend {
            v[word] |= 1 << bit;
        } else {
            v[word] &= !(1 << bit);
        }
    }

    pub fn is_spi_pending(&self, intid: usize) -> bool {
        if intid < 32 || intid >= self.num_intids() {
            return false;
        }
        let word = intid / 32;
        let bit = intid % 32;
        (self.pending.read().unwrap()[word] >> bit) & 1 != 0
    }

    pub fn set_spi_active(&self, intid: usize, act: bool) {
        if intid < 32 || intid >= self.num_intids() {
            return;
        }
        let word = intid / 32;
        let bit = intid % 32;
        let mut v = self.active.write().unwrap();
        if act {
            v[word] |= 1 << bit;
        } else {
            v[word] &= !(1 << bit);
        }
    }

    pub fn set_spi_priority(&self, intid: usize, prio: u8) {
        if intid >= self.num_intids() {
            return;
        }
        self.priority.write().unwrap()[intid] = prio;
    }

    pub fn spi_priority(&self, intid: usize) -> u8 {
        if intid >= self.num_intids() {
            return 0xFF;
        }
        self.priority.read().unwrap()[intid]
    }

    pub fn set_spi_target(&self, intid: usize, cpu: u8) {
        if intid < 32 || intid >= self.num_intids() {
            return;
        }
        self.target_cpu.write().unwrap()[intid - 32] = cpu;
    }

    pub fn spi_target(&self, intid: usize) -> u8 {
        if intid < 32 || intid >= self.num_intids() {
            return 0;
        }
        self.target_cpu.read().unwrap()[intid - 32]
    }

    /// Find the highest-priority pending+enabled SPI targeted at `cpu`.
    /// Returns (intid, priority) or None.
    pub fn highest_pending_spi(&self, cpu: u8) -> Option<(usize, u8)> {
        if !self.enabled {
            return None;
        }
        let enable = self.enable.read().unwrap();
        let pending = self.pending.read().unwrap();
        let active = self.active.read().unwrap();
        let prio = self.priority.read().unwrap();
        let target = self.target_cpu.read().unwrap();

        let mut best: Option<(usize, u8)> = None;
        for intid in 32..self.num_intids() {
            let w = intid / 32;
            let b = intid % 32;
            let mask = 1u32 << b;
            if enable[w] & mask == 0 {
                continue;
            }
            if pending[w] & mask == 0 {
                continue;
            }
            if active[w] & mask != 0 {
                continue;
            }
            if target[intid - 32] != cpu {
                continue;
            }
            let p = prio[intid];
            if best.is_none() || p < best.unwrap().1 {
                best = Some((intid, p));
            }
        }
        best
    }

    pub fn read_reg(&self, offset: u64) -> u64 {
        match offset {
            0x0000 => {
                if self.enabled {
                    1
                } else {
                    0
                }
            }
            0x0004 => {
                let it_lines = ((self.num_intids() + 31) / 32 - 1) as u64;
                it_lines & 0x1F
            }
            o if (0x0100..0x0180).contains(&o) => {
                let word = ((o - 0x0100) / 4) as usize;
                let v = self.enable.read().unwrap();
                if word < v.len() {
                    v[word] as u64
                } else {
                    0
                }
            }
            o if (0x0200..0x0280).contains(&o) => {
                let word = ((o - 0x0200) / 4) as usize;
                let v = self.pending.read().unwrap();
                if word < v.len() {
                    v[word] as u64
                } else {
                    0
                }
            }
            o if (0x0400..0x0800).contains(&o) => {
                let idx = (o - 0x0400) as usize;
                let prio = self.priority.read().unwrap();
                if idx < prio.len() {
                    prio[idx] as u64
                } else {
                    0xFF
                }
            }
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, offset: u64, val: u64) {
        match offset {
            0x0000 => self.enabled = val & 1 != 0,
            o if (0x0100..0x0180).contains(&o) => {
                let word = ((o - 0x0100) / 4) as usize;
                let mut v = self.enable.write().unwrap();
                if word < v.len() {
                    v[word] |= val as u32;
                }
            }
            o if (0x0180..0x0200).contains(&o) => {
                let word = ((o - 0x0180) / 4) as usize;
                let mut v = self.enable.write().unwrap();
                if word < v.len() {
                    v[word] &= !(val as u32);
                }
            }
            o if (0x0200..0x0280).contains(&o) => {
                let word = ((o - 0x0200) / 4) as usize;
                let mut v = self.pending.write().unwrap();
                if word < v.len() {
                    v[word] |= val as u32;
                }
            }
            o if (0x0280..0x0300).contains(&o) => {
                let word = ((o - 0x0280) / 4) as usize;
                let mut v = self.pending.write().unwrap();
                if word < v.len() {
                    v[word] &= !(val as u32);
                }
            }
            o if (0x0400..0x0800).contains(&o) => {
                let idx = (o - 0x0400) as usize;
                let mut prio = self.priority.write().unwrap();
                if idx < prio.len() {
                    prio[idx] = val as u8;
                }
            }
            o if (0x6000..0x8000).contains(&o) => {
                let spi_idx = ((o - 0x6000) / 8) as usize;
                if spi_idx < self.num_spis {
                    self.target_cpu.write().unwrap()[spi_idx] = (val & 0xFF) as u8;
                }
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.enabled = false;
        self.enable.write().unwrap().fill(0);
        self.pending.write().unwrap().fill(0);
        self.active.write().unwrap().fill(0);
        self.priority.write().unwrap().fill(0xFF);
        self.target_cpu.write().unwrap().fill(0);
    }
}
