//! SyncBus — 时序同步总线
//!
//! 协调混合精度仿真中不同 TimingModel 的全局时序。

use crate::error::OmniResult;
use crate::unit::UnitId;

pub struct SyncParticipant {
    pub unit_id: UnitId,
}

pub struct SyncBus {
    pub id:              String,
    pub participants:    Vec<SyncParticipant>,
    pub time_quantum_ns: u64,
    current_time_ns:     u64,
}

impl SyncBus {
    pub fn new(id: impl Into<String>, time_quantum_ns: u64) -> Self {
        Self { id: id.into(), participants: Vec::new(),
               time_quantum_ns, current_time_ns: 0 }
    }

    pub fn add_participant(&mut self, unit_id: UnitId) {
        self.participants.push(SyncParticipant { unit_id });
    }

    pub fn current_time_ns(&self) -> u64 { self.current_time_ns }

    /// 推进一个时间量子
    pub fn tick(&mut self) -> OmniResult<u64> {
        self.current_time_ns += self.time_quantum_ns;
        Ok(self.current_time_ns)
    }
}
