//! Backend 层 — 执行维度与时序维度抽象

pub mod execution;
pub mod timing;

pub use execution::{BackendEvent, BackendKind, DebugBackend, ExecutionBackend, StepResult};
pub use timing::{TimingKind, TimingModel};
