//! GICv3 Redistributor (GICR) — per-CPU
//!
//! Manages SGIs (0..15) and PPIs (16..31) for one PE.
//! Memory-mapped region: two 64KiB frames per PE (RD_base + SGI_base).
//!
//! SGI_base offsets (relative to frame 1):
//!   0x0100  GICR_ISENABLER0  — set-enable for INTIDs 0..31
//!   0x0180  GICR_ICENABLER0  — clear-enable
//!   0x0200  GICR_ISPENDR0    — set-pending
//!   0x0280  GICR_ICPENDR0    — clear-pending
//!   0x0400  GICR_IPRIORITYR  — priority (1 byte per INTID, INTIDs 0..31)

use std::sync::RwLock;

pub struct Gicr {
    cpu_id: u8,
    enable: RwLock<u32>,
    pending: RwLock<u32>,
    active: RwLock<u32>,
    priority: RwLock<[u8; 32]>,
}

impl Gicr {
    pub fn new(cpu_id: u8) -> Self {
        Self {
            cpu_id,
            enable: RwLock::new(0),
            pending: RwLock::new(0),
            active: RwLock::new(0),
            priority: RwLock::new([0xFF; 32]),
        }
    }

    pub fn cpu_id(&self) -> u8 {
        self.cpu_id
    }

    pub fn set_enable(&self, intid: usize, en: bool) {
        if intid >= 32 {
            return;
        }
        let mut v = self.enable.write().unwrap();
        if en {
            *v |= 1 << intid;
        } else {
            *v &= !(1 << intid);
        }
    }

    pub fn is_enabled(&self, intid: usize) -> bool {
        if intid >= 32 {
            return false;
        }
        (*self.enable.read().unwrap() >> intid) & 1 != 0
    }

    pub fn set_pending(&self, intid: usize, pend: bool) {
        if intid >= 32 {
            return;
        }
        let mut v = self.pending.write().unwrap();
        if pend {
            *v |= 1 << intid;
        } else {
            *v &= !(1 << intid);
        }
    }

    pub fn is_pending(&self, intid: usize) -> bool {
        if intid >= 32 {
            return false;
        }
        (*self.pending.read().unwrap() >> intid) & 1 != 0
    }

    pub fn set_active(&self, intid: usize, act: bool) {
        if intid >= 32 {
            return;
        }
        let mut v = self.active.write().unwrap();
        if act {
            *v |= 1 << intid;
        } else {
            *v &= !(1 << intid);
        }
    }

    pub fn set_priority(&self, intid: usize, prio: u8) {
        if intid >= 32 {
            return;
        }
        self.priority.write().unwrap()[intid] = prio;
    }

    pub fn priority(&self, intid: usize) -> u8 {
        if intid >= 32 {
            return 0xFF;
        }
        self.priority.read().unwrap()[intid]
    }

    /// Find highest-priority pending+enabled SGI/PPI on this PE.
    pub fn highest_pending(&self) -> Option<(usize, u8)> {
        let enable = *self.enable.read().unwrap();
        let pending = *self.pending.read().unwrap();
        let active = *self.active.read().unwrap();
        let prio = self.priority.read().unwrap();

        let candidates = enable & pending & !active;
        if candidates == 0 {
            return None;
        }

        let mut best: Option<(usize, u8)> = None;
        for intid in 0..32 {
            if candidates & (1 << intid) == 0 {
                continue;
            }
            let p = prio[intid];
            if best.is_none() || p < best.unwrap().1 {
                best = Some((intid, p));
            }
        }
        best
    }

    /// Read SGI_base register (offset within the SGI frame, i.e., second 64K frame)
    pub fn read_sgi_reg(&self, offset: u64) -> u64 {
        match offset {
            0x0100 => *self.enable.read().unwrap() as u64,
            0x0200 => *self.pending.read().unwrap() as u64,
            o if (0x0400..0x0420).contains(&o) => {
                let idx = (o - 0x0400) as usize;
                if idx < 32 {
                    self.priority.read().unwrap()[idx] as u64
                } else {
                    0xFF
                }
            }
            _ => 0,
        }
    }

    /// Write SGI_base register
    pub fn write_sgi_reg(&self, offset: u64, val: u64) {
        match offset {
            0x0100 => *self.enable.write().unwrap() |= val as u32,
            0x0180 => *self.enable.write().unwrap() &= !(val as u32),
            0x0200 => *self.pending.write().unwrap() |= val as u32,
            0x0280 => *self.pending.write().unwrap() &= !(val as u32),
            o if (0x0400..0x0420).contains(&o) => {
                let idx = (o - 0x0400) as usize;
                if idx < 32 {
                    self.priority.write().unwrap()[idx] = val as u8;
                }
            }
            _ => {}
        }
    }

    pub fn reset(&self) {
        *self.enable.write().unwrap() = 0;
        *self.pending.write().unwrap() = 0;
        *self.active.write().unwrap() = 0;
        self.priority.write().unwrap().fill(0xFF);
    }
}
