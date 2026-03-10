//! omnisim-core — OmniSim L1+L2 接口定义
//!
//! 零业务依赖（仅允许 std + thiserror + bitflags）。
//! 所有其他 crate 的唯一共同依赖。

pub mod error;
pub mod unit;
pub mod backend;
pub mod mem;
pub mod irq;
pub mod dataflow;
pub mod bus;
pub mod security;
pub mod coherence;
pub mod network;
pub mod registry;
pub mod snapshot;

pub mod prelude {
    pub use crate::error::{OmniError, OmniResult};
    pub use crate::unit::{Unit, UnitId, UnitCaps, RealizeCtx, BootStep};
    pub use crate::backend::execution::ExecutionBackend;
    pub use crate::backend::timing::{TimingModel, TimingKind};
    pub use crate::mem::{MemTarget, MemSpace, MemAttrs, MemAccessSize, MemAccess};
    pub use crate::irq::{IrqSink, IrqLine, IrqSource, MsiSender};
    pub use crate::dataflow::{DataEndpoint, DataDesc, DataOp};
    pub use crate::registry::UnitRegistry;
    pub use crate::snapshot::{UnitSnapshot, PlatformSnapshot};
}
