//! RISC-V PLIC (Platform-Level Interrupt Controller)
//!
//! Memory map (SiFive spec):
//!   0x000000 ..= 0x000FFF  priority[1..=1023]  (4 bytes each, source 0 reserved)
//!   0x001000 ..= 0x00107F  pending[0..=31]      (32 × 4-byte words, bit per source)
//!   0x002000 ..= 0x1F1FFF  enable[ctx][0..=31]  (per-context enable bits, 0x80 stride)
//!   0x200000 ..= 0x3FFFFF  threshold/claim      (per-context, 0x1000 stride)
//!     +0x000  threshold
//!     +0x004  claim/complete
//!
//! Total size: 0x400000 (4 MiB)

use std::sync::{Arc, RwLock};
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::irq::{IrqLineSource, IrqSink};
use rhetsim_core::mem::{MemTarget, MemAccessSize};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};

const PLIC_SIZE: u64 = 0x400_0000;
const MAX_SOURCES: usize = 1024;
const MAX_CONTEXTS: usize = 15872; // spec max
const PRIORITY_BASE: u64 = 0x0;
const PENDING_BASE: u64 = 0x1000;
const ENABLE_BASE: u64 = 0x2000;
const ENABLE_STRIDE: u64 = 0x80;
const CONTEXT_BASE: u64 = 0x20_0000;
const CONTEXT_STRIDE: u64 = 0x1000;

struct PlicState {
    priority: Vec<u32>,
    pending: Vec<u32>,
    enable: Vec<Vec<u32>>,
    threshold: Vec<u32>,
    claimed: Vec<Vec<u32>>,
}

pub struct Plic {
    id: UnitId,
    base: u64,
    num_sources: usize,
    num_contexts: usize,
    state: RwLock<PlicState>,
}

impl Plic {
    pub fn new(id: &str, num_sources: usize, num_contexts: usize, base: u64) -> Self {
        assert!(num_sources > 0 && num_sources <= MAX_SOURCES);
        assert!(num_contexts > 0 && num_contexts <= MAX_CONTEXTS);
        // Source IDs are 1..=num_sources and bit 0 is reserved, so the
        // pending/enable/claimed bitmaps need one extra bit for the last
        // source when num_sources is a multiple of 32.
        let words = (num_sources + 32) / 32;
        Self {
            id: UnitId::new(id),
            base,
            num_sources,
            num_contexts,
            state: RwLock::new(PlicState {
                priority: vec![0u32; num_sources + 1],
                pending: vec![0u32; words],
                enable: vec![vec![0u32; words]; num_contexts],
                threshold: vec![0u32; num_contexts],
                claimed: vec![vec![0u32; words]; num_contexts],
            }),
        }
    }

    pub fn pending_irq(&self, context: usize) -> Option<u32> {
        let state = self.state.read().unwrap();
        if context >= self.num_contexts { return None; }
        let threshold = state.threshold[context];
        let words = (self.num_sources + 31) / 32;
        let mut best_irq: Option<u32> = None;
        let mut best_prio = 0u32;

        for w in 0..words {
            let candidates = state.pending[w] & state.enable[context][w] & !state.claimed[context][w];
            if candidates == 0 { continue; }
            for bit in 0..32 {
                if candidates & (1 << bit) == 0 { continue; }
                let irq = (w * 32 + bit) as u32;
                if irq == 0 || irq as usize > self.num_sources { continue; }
                let prio = state.priority[irq as usize];
                if prio > threshold && prio > best_prio {
                    best_prio = prio;
                    best_irq = Some(irq);
                }
            }
        }
        best_irq
    }
}

impl Unit for Plic {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "riscv-plic" }
    fn caps(&self) -> UnitCaps { UnitCaps::ADDRESSABLE | UnitCaps::IRQ_SINK }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }

    fn reset(&mut self) -> OmniResult<()> {
        let mut state = self.state.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        state.priority.fill(0);
        state.pending.fill(0);
        state.threshold.fill(0);
        for v in &mut state.enable { v.fill(0); }
        for v in &mut state.claimed { v.fill(0); }
        Ok(())
    }
}

impl MemTarget for Plic {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { self.base }
    fn size(&self) -> u64 { PLIC_SIZE }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        let state = self.state.read().map_err(|e| OmniError::Internal(e.to_string()))?;

        // Priority registers
        if offset < PENDING_BASE {
            let idx = (offset / 4) as usize;
            if idx > 0 && idx <= self.num_sources {
                return Ok(state.priority[idx] as u64);
            }
            return Ok(0);
        }

        // Pending registers
        if offset >= PENDING_BASE && offset < ENABLE_BASE {
            let word = ((offset - PENDING_BASE) / 4) as usize;
            if word < state.pending.len() {
                return Ok(state.pending[word] as u64);
            }
            return Ok(0);
        }

        // Enable registers
        if offset >= ENABLE_BASE && offset < CONTEXT_BASE {
            let rel = offset - ENABLE_BASE;
            let ctx = (rel / ENABLE_STRIDE) as usize;
            let word = ((rel % ENABLE_STRIDE) / 4) as usize;
            if ctx < self.num_contexts && word < state.enable[ctx].len() {
                return Ok(state.enable[ctx][word] as u64);
            }
            return Ok(0);
        }

        // Context threshold/claim
        if offset >= CONTEXT_BASE {
            let rel = offset - CONTEXT_BASE;
            let ctx = (rel / CONTEXT_STRIDE) as usize;
            let reg = rel % CONTEXT_STRIDE;
            if ctx < self.num_contexts {
                match reg {
                    0 => return Ok(state.threshold[ctx] as u64),
                    4 => {
                        // Claim: return highest-priority pending+enabled IRQ
                        drop(state);
                        if let Some(irq) = self.pending_irq(ctx) {
                            let mut state = self.state.write().map_err(|e| OmniError::Internal(e.to_string()))?;
                            let w = irq as usize / 32;
                            let b = irq as usize % 32;
                            state.claimed[ctx][w] |= 1 << b;
                            return Ok(irq as u64);
                        }
                        return Ok(0);
                    }
                    _ => {}
                }
            }
        }

        Ok(0)
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        let mut state = self.state.write().map_err(|e| OmniError::Internal(e.to_string()))?;

        // Priority
        if offset < PENDING_BASE {
            let idx = (offset / 4) as usize;
            if idx > 0 && idx <= self.num_sources {
                state.priority[idx] = val as u32;
            }
            return Ok(());
        }

        // Pending: read-only (set via IrqSink::raise)
        if offset >= PENDING_BASE && offset < ENABLE_BASE {
            return Ok(());
        }

        // Enable
        if offset >= ENABLE_BASE && offset < CONTEXT_BASE {
            let rel = offset - ENABLE_BASE;
            let ctx = (rel / ENABLE_STRIDE) as usize;
            let word = ((rel % ENABLE_STRIDE) / 4) as usize;
            if ctx < self.num_contexts && word < state.enable[ctx].len() {
                state.enable[ctx][word] = val as u32;
            }
            return Ok(());
        }

        // Context threshold/complete
        if offset >= CONTEXT_BASE {
            let rel = offset - CONTEXT_BASE;
            let ctx = (rel / CONTEXT_STRIDE) as usize;
            let reg = rel % CONTEXT_STRIDE;
            if ctx < self.num_contexts {
                match reg {
                    0 => state.threshold[ctx] = val as u32,
                    4 => {
                        // Complete: deactivate the source. Keep the pending
                        // bit as the level state: if the source is still
                        // asserted (e.g. UART RX FIFO not empty), the next
                        // pending_irq() must deliver it again.
                        let irq = val as u32;
                        if (irq as usize) <= self.num_sources {
                            let w = irq as usize / 32;
                            let b = irq as usize % 32;
                            state.claimed[ctx][w] &= !(1 << b);
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

impl IrqSink for Plic {
    fn raise(&self, irq_num: u32) -> OmniResult<()> {
        if irq_num == 0 || irq_num as usize > self.num_sources {
            return Err(OmniError::IrqNotFound(format!("source {} out of range", irq_num)));
        }
        let mut state = self.state.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        let w = irq_num as usize / 32;
        let b = irq_num as usize % 32;
        state.pending[w] |= 1 << b;
        Ok(())
    }

    fn lower(&self, irq_num: u32) -> OmniResult<()> {
        if irq_num == 0 || irq_num as usize > self.num_sources {
            return Err(OmniError::IrqNotFound(format!("source {} out of range", irq_num)));
        }
        let mut state = self.state.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        let w = irq_num as usize / 32;
        let b = irq_num as usize % 32;
        state.pending[w] &= !(1 << b);
        Ok(())
    }

    fn irq_count(&self) -> u32 { self.num_sources as u32 }
}

/// 某个 PLIC context 的输出线，可由 RISC-V CPU 轮询同步到 mip.MEIP/SEIP。
pub struct PlicContextLine {
    plic: Arc<Plic>,
    context: usize,
}

impl PlicContextLine {
    pub fn new(plic: Arc<Plic>, context: usize) -> Self {
        Self { plic, context }
    }
}

impl IrqLineSource for PlicContextLine {
    fn is_asserted(&self) -> bool {
        self.plic.pending_irq(self.context).is_some()
    }
}

pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_read_write() {
        let plic = Plic::new("plic0", 32, 2, 0x0C00_0000);
        plic.write(4, MemAccessSize::U32, 7).unwrap(); // source 1 priority = 7
        assert_eq!(plic.read(4, MemAccessSize::U32).unwrap(), 7);
    }

    #[test]
    fn test_raise_pending() {
        let plic = Plic::new("plic0", 32, 2, 0x0C00_0000);
        plic.raise(1).unwrap();
        let pending = plic.read(PENDING_BASE, MemAccessSize::U32).unwrap();
        assert_eq!(pending & 0x2, 0x2); // bit 1 set
    }

    #[test]
    fn test_claim_complete() {
        let plic = Plic::new("plic0", 32, 2, 0x0C00_0000);

        // Set priority for source 1
        plic.write(4, MemAccessSize::U32, 5).unwrap();
        // Enable source 1 for context 0
        plic.write(ENABLE_BASE, MemAccessSize::U32, 0x2).unwrap(); // bit 1
        // Set threshold to 0 for context 0
        plic.write(CONTEXT_BASE, MemAccessSize::U32, 0).unwrap();

        // Raise source 1
        plic.raise(1).unwrap();

        // Check pending_irq
        assert_eq!(plic.pending_irq(0), Some(1));

        // Claim
        let claimed = plic.read(CONTEXT_BASE + 4, MemAccessSize::U32).unwrap();
        assert_eq!(claimed, 1);

        // After claim, should not be claimable again
        assert_eq!(plic.pending_irq(0), None);

        // Source deasserts while the interrupt is claimed, then complete.
        plic.lower(1).unwrap();
        plic.write(CONTEXT_BASE + 4, MemAccessSize::U32, 1).unwrap();

        // Pending should be cleared
        let pending = plic.read(PENDING_BASE, MemAccessSize::U32).unwrap();
        assert_eq!(pending & 0x2, 0);
    }

    #[test]
    fn test_level_reattach_after_complete() {
        let plic = Plic::new("plic0", 32, 2, 0x0C00_0000);
        plic.write(4, MemAccessSize::U32, 1).unwrap();
        plic.write(ENABLE_BASE, MemAccessSize::U32, 0x2).unwrap();
        plic.raise(1).unwrap();

        // Claim + complete while the source line is still asserted.
        assert_eq!(plic.read(CONTEXT_BASE + 4, MemAccessSize::U32).unwrap(), 1);
        plic.write(CONTEXT_BASE + 4, MemAccessSize::U32, 1).unwrap();

        // Level is still high, so the PLIC must present it again.
        assert_eq!(plic.pending_irq(0), Some(1));

        // Deasserting the source clears it.
        plic.lower(1).unwrap();
        assert_eq!(plic.pending_irq(0), None);
    }

    #[test]
    fn test_context_line() {
        let plic = Arc::new(Plic::new("plic0", 32, 2, 0x0C00_0000));
        // Enable source 1 for context 1 only.
        plic.write(4, MemAccessSize::U32, 1).unwrap();
        plic.write(ENABLE_BASE + ENABLE_STRIDE, MemAccessSize::U32, 0x2).unwrap();
        plic.raise(1).unwrap();

        let s_line = PlicContextLine::new(plic.clone(), 1);
        assert!(s_line.is_asserted());

        let m_line = PlicContextLine::new(plic, 0);
        assert!(!m_line.is_asserted());
    }

    #[test]
    fn test_priority_ordering() {
        let plic = Plic::new("plic0", 32, 1, 0x0C00_0000);

        // Source 1 priority 3, source 2 priority 7
        plic.write(4, MemAccessSize::U32, 3).unwrap();
        plic.write(8, MemAccessSize::U32, 7).unwrap();

        // Enable both for context 0
        plic.write(ENABLE_BASE, MemAccessSize::U32, 0x6).unwrap(); // bits 1,2

        // Raise both
        plic.raise(1).unwrap();
        plic.raise(2).unwrap();

        // Should claim source 2 (higher priority)
        assert_eq!(plic.pending_irq(0), Some(2));
    }

    #[test]
    fn test_threshold() {
        let plic = Plic::new("plic0", 32, 1, 0x0C00_0000);

        plic.write(4, MemAccessSize::U32, 3).unwrap(); // source 1 prio 3
        plic.write(ENABLE_BASE, MemAccessSize::U32, 0x2).unwrap();
        plic.raise(1).unwrap();

        // Threshold = 3 → source 1 (prio 3) should NOT fire (must be > threshold)
        plic.write(CONTEXT_BASE, MemAccessSize::U32, 3).unwrap();
        assert_eq!(plic.pending_irq(0), None);

        // Threshold = 2 → should fire
        plic.write(CONTEXT_BASE, MemAccessSize::U32, 2).unwrap();
        assert_eq!(plic.pending_irq(0), Some(1));
    }

    #[test]
    fn test_irq_source_zero_reserved() {
        let plic = Plic::new("plic0", 32, 1, 0x0C00_0000);
        assert!(plic.raise(0).is_err());
    }

    #[test]
    fn test_reset() {
        let mut plic = Plic::new("plic0", 32, 1, 0x0C00_0000);
        plic.write(4, MemAccessSize::U32, 7).unwrap();
        plic.raise(1).unwrap();
        plic.reset().unwrap();
        assert_eq!(plic.read(4, MemAccessSize::U32).unwrap(), 0);
        assert_eq!(plic.pending_irq(0), None);
    }
}
