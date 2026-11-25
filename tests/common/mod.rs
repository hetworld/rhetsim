//! 测试工具 — MinimalPlatform / MockBackend

use rhetsim_core::error::OmniResult;
use rhetsim_core::unit::{Unit, UnitId, UnitCaps, RealizeCtx};

/// 最小平台：用于集成测试
pub struct MinimalPlatform;

impl MinimalPlatform {
    pub fn new() -> Self { Self }
}

/// Mock ExecutionBackend
pub struct MockBackend {
    pub exec_count: u64,
}

impl MockBackend {
    pub fn new() -> Self { Self { exec_count: 0 } }
}
