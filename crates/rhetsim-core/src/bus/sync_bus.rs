//! SyncBus — 时序同步总线
//!
//! 协调混合精度仿真中不同 TimingModel 的全局时序。

use crate::error::OmniResult;
use crate::unit::UnitId;

/// 参与全局时序同步的 Unit
pub struct SyncParticipant {
    /// 参与同步的 Unit ID
    pub unit_id: UnitId,
}

/// 时序同步总线
pub struct SyncBus {
    /// 总线 ID
    pub id: String,
    /// 参与同步的 Unit 列表
    pub participants: Vec<SyncParticipant>,
    /// 时间量子（纳秒）
    pub time_quantum_ns: u64,
    current_time_ns: u64,
}

impl SyncBus {
    /// 创建指定时间量子的同步总线
    pub fn new(id: impl Into<String>, time_quantum_ns: u64) -> Self {
        Self {
            id: id.into(),
            participants: Vec::new(),
            time_quantum_ns,
            current_time_ns: 0,
        }
    }

    /// 注册一个同步参与者
    pub fn add_participant(&mut self, unit_id: UnitId) {
        self.participants.push(SyncParticipant { unit_id });
    }

    /// 当前全局时间（纳秒）
    pub fn current_time_ns(&self) -> u64 {
        self.current_time_ns
    }

    /// 推进一个时间量子
    pub fn tick(&mut self) -> OmniResult<u64> {
        self.current_time_ns += self.time_quantum_ns;
        Ok(self.current_time_ns)
    }
}
