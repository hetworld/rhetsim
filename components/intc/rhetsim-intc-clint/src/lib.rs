//! RISC-V CLINT (Core Local Interruptor)
//!
//! Memory map (per-hart, up to 4095 harts):
//!   0x0000 ..= 0x3FFF  msip[hart]      (4 bytes each, bit 0 = MSIP)
//!   0x4000 ..= 0xBFF7  mtimecmp[hart]   (8 bytes each)
//!   0xBFF8 ..= 0xBFFF  mtime            (8 bytes, shared)
//!
//! Total size: 0xC000 (48 KiB)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::irq::{IrqLineSource, IrqSink};
use rhetsim_core::mem::{MemTarget, MemAccessSize};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};

const MSIP_BASE: u64 = 0x0000;
const MTIMECMP_BASE: u64 = 0x4000;
const MTIME_OFFSET: u64 = 0xBFF8;
const CLINT_SIZE: u64 = 0xC000;
const MAX_HARTS: usize = 4095;

pub struct Clint {
    id: UnitId,
    base: u64,
    num_harts: usize,
    mtime: AtomicU64,
    msip: RwLock<Vec<u32>>,
    mtimecmp: RwLock<Vec<u64>>,
}

impl Clint {
    pub fn new(id: &str, num_harts: usize, base: u64) -> Self {
        assert!(num_harts > 0 && num_harts <= MAX_HARTS);
        Self {
            id: UnitId::new(id),
            base,
            num_harts,
            mtime: AtomicU64::new(0),
            msip: RwLock::new(vec![0u32; num_harts]),
            mtimecmp: RwLock::new(vec![u64::MAX; num_harts]),
        }
    }

    pub fn tick(&self, delta: u64) {
        self.mtime.fetch_add(delta, Ordering::Relaxed);
    }

    pub fn mtime(&self) -> u64 {
        self.mtime.load(Ordering::Relaxed)
    }

    pub fn mtimecmp(&self, hart: usize) -> u64 {
        if hart >= self.num_harts { return u64::MAX; }
        self.mtimecmp.read().unwrap()[hart]
    }

    pub fn timer_pending(&self, hart: usize) -> bool {
        if hart >= self.num_harts { return false; }
        let mtime = self.mtime.load(Ordering::Relaxed);
        let mtimecmp = self.mtimecmp.read().unwrap();
        mtime >= mtimecmp[hart]
    }

    pub fn sw_pending(&self, hart: usize) -> bool {
        if hart >= self.num_harts { return false; }
        let msip = self.msip.read().unwrap();
        msip[hart] & 1 != 0
    }
}

/// CLINT 的 timer/software 输出线，可由 RISC-V CPU 轮询同步到 mip。
pub struct ClintLine {
    clint: Arc<Clint>,
    hart: usize,
    timer: bool,
}

impl ClintLine {
    pub fn new(clint: Arc<Clint>, hart: usize, timer: bool) -> Self {
        Self { clint, hart, timer }
    }
}

impl IrqLineSource for ClintLine {
    fn is_asserted(&self) -> bool {
        if self.timer {
            self.clint.timer_pending(self.hart)
        } else {
            self.clint.sw_pending(self.hart)
        }
    }
}

impl Unit for Clint {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "riscv-clint" }
    fn caps(&self) -> UnitCaps { UnitCaps::ADDRESSABLE }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }

    fn reset(&mut self) -> OmniResult<()> {
        self.mtime.store(0, Ordering::Relaxed);
        let mut msip = self.msip.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        msip.fill(0);
        let mut mtimecmp = self.mtimecmp.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        mtimecmp.fill(u64::MAX);
        Ok(())
    }
}

impl MemTarget for Clint {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { self.base }
    fn size(&self) -> u64 { CLINT_SIZE }

    fn read(&self, offset: u64, size: MemAccessSize) -> OmniResult<u64> {
        if offset >= MTIME_OFFSET && offset < MTIME_OFFSET + 8 {
            let mtime = self.mtime.load(Ordering::Relaxed);
            let byte_off = (offset - MTIME_OFFSET) as usize;
            return Ok(extract_bytes(mtime, byte_off, size));
        }

        if offset >= MTIMECMP_BASE && offset < MTIME_OFFSET {
            let rel = offset - MTIMECMP_BASE;
            let hart = (rel / 8) as usize;
            if hart < self.num_harts {
                let mtimecmp = self.mtimecmp.read().map_err(|e| OmniError::Internal(e.to_string()))?;
                let byte_off = (rel % 8) as usize;
                return Ok(extract_bytes(mtimecmp[hart], byte_off, size));
            }
        }

        if offset >= MSIP_BASE && offset < MTIMECMP_BASE {
            let rel = offset - MSIP_BASE;
            let hart = (rel / 4) as usize;
            if hart < self.num_harts {
                let msip = self.msip.read().map_err(|e| OmniError::Internal(e.to_string()))?;
                let byte_off = (rel % 4) as usize;
                return Ok(extract_bytes(msip[hart] as u64, byte_off, size));
            }
        }

        Ok(0)
    }

    fn write(&self, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()> {
        if offset >= MTIME_OFFSET && offset < MTIME_OFFSET + 8 {
            let byte_off = (offset - MTIME_OFFSET) as usize;
            let old = self.mtime.load(Ordering::Relaxed);
            let new = deposit_bytes(old, byte_off, size, val);
            self.mtime.store(new, Ordering::Relaxed);
            return Ok(());
        }

        if offset >= MTIMECMP_BASE && offset < MTIME_OFFSET {
            let rel = offset - MTIMECMP_BASE;
            let hart = (rel / 8) as usize;
            if hart < self.num_harts {
                let mut mtimecmp = self.mtimecmp.write().map_err(|e| OmniError::Internal(e.to_string()))?;
                let byte_off = (rel % 8) as usize;
                mtimecmp[hart] = deposit_bytes(mtimecmp[hart], byte_off, size, val);
            }
            return Ok(());
        }

        if offset >= MSIP_BASE && offset < MTIMECMP_BASE {
            let rel = offset - MSIP_BASE;
            let hart = (rel / 4) as usize;
            if hart < self.num_harts {
                let mut msip = self.msip.write().map_err(|e| OmniError::Internal(e.to_string()))?;
                msip[hart] = (val as u32) & 1;
            }
            return Ok(());
        }

        Ok(())
    }
}

impl IrqSink for Clint {
    fn raise(&self, irq_num: u32) -> OmniResult<()> {
        let hart = irq_num as usize;
        if hart >= self.num_harts {
            return Err(OmniError::IrqNotFound(format!("hart {} out of range", hart)));
        }
        let mut msip = self.msip.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        msip[hart] |= 1;
        Ok(())
    }

    fn lower(&self, irq_num: u32) -> OmniResult<()> {
        let hart = irq_num as usize;
        if hart >= self.num_harts {
            return Err(OmniError::IrqNotFound(format!("hart {} out of range", hart)));
        }
        let mut msip = self.msip.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        msip[hart] &= !1;
        Ok(())
    }

    fn irq_count(&self) -> u32 { self.num_harts as u32 }
}

fn extract_bytes(val: u64, byte_off: usize, size: MemAccessSize) -> u64 {
    let shift = byte_off * 8;
    match size {
        MemAccessSize::U8  => (val >> shift) & 0xFF,
        MemAccessSize::U16 => (val >> shift) & 0xFFFF,
        MemAccessSize::U32 => (val >> shift) & 0xFFFF_FFFF,
        MemAccessSize::U64 | MemAccessSize::U128 => val >> shift,
    }
}

fn deposit_bytes(old: u64, byte_off: usize, size: MemAccessSize, val: u64) -> u64 {
    let shift = byte_off * 8;
    let mask: u64 = match size {
        MemAccessSize::U8  => 0xFF << shift,
        MemAccessSize::U16 => 0xFFFF << shift,
        MemAccessSize::U32 => 0xFFFF_FFFF << shift,
        MemAccessSize::U64 | MemAccessSize::U128 => !0u64,
    };
    (old & !mask) | ((val << shift) & mask)
}

pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtime_tick() {
        let clint = Clint::new("clint0", 1, 0x200_0000);
        assert_eq!(clint.mtime(), 0);
        clint.tick(100);
        assert_eq!(clint.mtime(), 100);
        clint.tick(50);
        assert_eq!(clint.mtime(), 150);
    }

    #[test]
    fn test_mtime_read_write() {
        let clint = Clint::new("clint0", 1, 0x200_0000);
        clint.write(MTIME_OFFSET, MemAccessSize::U64, 0x12345678_ABCDEF00).unwrap();
        let val = clint.read(MTIME_OFFSET, MemAccessSize::U64).unwrap();
        assert_eq!(val, 0x12345678_ABCDEF00);
    }

    #[test]
    fn test_mtimecmp_timer_pending() {
        let clint = Clint::new("clint0", 2, 0x200_0000);
        assert!(!clint.timer_pending(0));

        clint.write(MTIMECMP_BASE, MemAccessSize::U64, 100).unwrap();
        assert!(!clint.timer_pending(0));

        clint.tick(100);
        assert!(clint.timer_pending(0));
        assert!(!clint.timer_pending(1));
    }

    #[test]
    fn test_msip() {
        let clint = Clint::new("clint0", 2, 0x200_0000);
        assert!(!clint.sw_pending(0));

        clint.write(MSIP_BASE, MemAccessSize::U32, 1).unwrap();
        assert!(clint.sw_pending(0));
        assert!(!clint.sw_pending(1));

        clint.write(MSIP_BASE + 4, MemAccessSize::U32, 1).unwrap();
        assert!(clint.sw_pending(1));

        clint.write(MSIP_BASE, MemAccessSize::U32, 0).unwrap();
        assert!(!clint.sw_pending(0));
    }

    #[test]
    fn test_irq_sink() {
        let clint = Clint::new("clint0", 2, 0x200_0000);
        clint.raise(0).unwrap();
        assert!(clint.sw_pending(0));
        clint.lower(0).unwrap();
        assert!(!clint.sw_pending(0));
    }

    #[test]
    fn test_reset() {
        let mut clint = Clint::new("clint0", 1, 0x200_0000);
        clint.tick(500);
        clint.write(MSIP_BASE, MemAccessSize::U32, 1).unwrap();
        clint.write(MTIMECMP_BASE, MemAccessSize::U64, 42).unwrap();

        clint.reset().unwrap();
        assert_eq!(clint.mtime(), 0);
        assert!(!clint.sw_pending(0));
        assert!(!clint.timer_pending(0));
    }

    #[test]
    fn test_partial_access() {
        let clint = Clint::new("clint0", 1, 0x200_0000);
        clint.write(MTIME_OFFSET, MemAccessSize::U32, 0xDEADBEEF).unwrap();
        let lo = clint.read(MTIME_OFFSET, MemAccessSize::U32).unwrap();
        assert_eq!(lo, 0xDEADBEEF);

        clint.write(MTIME_OFFSET + 4, MemAccessSize::U32, 0xCAFEBABE).unwrap();
        let full = clint.read(MTIME_OFFSET, MemAccessSize::U64).unwrap();
        assert_eq!(full, 0xCAFEBABE_DEADBEEF);
    }
}
