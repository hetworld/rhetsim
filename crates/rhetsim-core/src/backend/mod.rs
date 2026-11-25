//! Backend 层 — 执行维度与时序维度抽象

pub mod execution;
pub mod timing;

pub use execution::{ExecutionBackend, DebugBackend, BackendKind, StepResult, BackendEvent};
pub use timing::{TimingModel, TimingKind};
