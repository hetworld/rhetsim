//! UnitThread — 单元执行线程

use omnisim_core::error::OmniResult;
use omnisim_core::unit::UnitId;

#[derive(Debug)]
pub enum CtrlCmd { Pause, Resume, Stop, Reset, SingleStep }

#[derive(Debug)]
pub enum UnitEvent {
    VmExit   { unit_id: UnitId, reason: u32, addr: u64 },
    MmioAccess { unit_id: UnitId, addr: u64, size: u8, is_write: bool, data: u64 },
    Halted   { unit_id: UnitId },
    Error    { unit_id: UnitId, msg: String },
}

pub struct UnitThreadHandle {
    pub unit_id: UnitId,
}
