//! rhetsim-core — RhetSim L1+L2 接口定义
//!
//! 零业务依赖（仅允许 std + thiserror + bitflags）。
//! 所有其他 crate 的唯一共同依赖。

pub mod backend;
pub mod bus;
pub mod coherence;
pub mod dataflow;
pub mod diag;
pub mod error;
pub mod irq;
pub mod mem;
pub mod network;
pub mod registry;
pub mod security;
pub mod snapshot;
pub mod unit;

/// 常用类型重导出
pub mod prelude {
    pub use crate::backend::execution::ExecutionBackend;
    pub use crate::backend::timing::{TimingKind, TimingModel};
    pub use crate::dataflow::{DataDesc, DataEndpoint, DataOp};
    pub use crate::error::{OmniError, OmniResult};
    pub use crate::irq::{IrqLine, IrqLineSource, IrqSink, IrqSource, MsiSender};
    pub use crate::mem::{MemAccess, MemAccessSize, MemAttrs, MemSpace, MemTarget};
    pub use crate::registry::UnitRegistry;
    pub use crate::snapshot::{PlatformSnapshot, UnitSnapshot};
    pub use crate::unit::{BootStep, RealizeCtx, Unit, UnitCaps, UnitId};
}
