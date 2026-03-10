//! VmExitDispatcher — VM Exit 处理流程

use omnisim_core::error::OmniResult;
use omnisim_core::unit::UnitId;

pub struct VmExitDispatcher;

impl VmExitDispatcher {
    pub fn new() -> Self { Self }

    pub fn dispatch(&self, unit_id: &UnitId, reason: u32, addr: u64) -> OmniResult<()> {
        tracing::debug!("VM exit: unit={} reason={} addr={:#x}", unit_id, reason, addr);
        Ok(())
    }
}

impl Default for VmExitDispatcher {
    fn default() -> Self { Self::new() }
}
