//! Cache 统一模型 — L1 / L2 / L3 由参数区分
//!
//! P0 阶段：功能仿真透明（访问直接穿透到 DRAM）
//! P1 阶段：LooseTiming，通过 capacity/latency/associativity 参数区分层级
//! P2 阶段：可按需拆分为独立 crate 支持 RTL 协同

use omnisim_core::error::OmniResult;

/// Cache 层级参数
#[derive(Debug, Clone)]
pub struct CacheParams {
    /// L1 / L2 / L3
    pub level:        u8,
    /// 容量（字节）
    pub capacity:     usize,
    /// 相联度
    pub associativity: usize,
    /// 行大小（字节）
    pub line_size:    usize,
    /// 访问延迟（周期，LooseTiming 使用）
    pub latency_cycles: u32,
}

impl CacheParams {
    pub fn l1() -> Self {
        Self { level: 1, capacity: 64*1024, associativity: 4,
               line_size: 64, latency_cycles: 4 }
    }
    pub fn l2() -> Self {
        Self { level: 2, capacity: 512*1024, associativity: 8,
               line_size: 64, latency_cycles: 12 }
    }
    pub fn l3() -> Self {
        Self { level: 3, capacity: 8*1024*1024, associativity: 16,
               line_size: 64, latency_cycles: 40 }
    }
}

pub struct Cache {
    pub params: CacheParams,
}

impl Cache {
    pub fn new(params: CacheParams) -> Self { Self { params } }
}

pub fn register() {}
