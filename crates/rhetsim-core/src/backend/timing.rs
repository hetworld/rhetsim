//! TimingModel / TimingKind — 时序模型接口
//!
//! 与 ExecutionBackend 正交，描述 Unit 如何参与全局时间推进。

use crate::error::OmniResult;

/// 时序模型接口 — 由 SyncBus 协调混合精度仿真
pub trait TimingModel: Send + Sync {
    /// Unit 在仿真时间 `now_ns` 时请求推进到哪个时间点
    fn next_event_ns(&self) -> u64;

    /// 通知 Unit 时间已推进到 `now_ns`
    fn advance(&mut self, now_ns: u64) -> OmniResult<()>;

    /// 时序精度类型
    fn timing_kind(&self) -> TimingKind;
}

/// 时序精度级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum TimingKind {
    /// 无时序（功能仿真，默认）
    #[default]
    Untimed,
    /// 松散时序（事件驱动，误差在时间量子内）
    Loose,
    /// 周期精确（每周期精确建模）
    CycleAccurate,
}
