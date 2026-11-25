//! InterconnectBus — 对等节点互联总线
//!
//! 覆盖：多卡 GPU 互联、NUMA 多路服务器、CXL 内存、片上 NoC。

use std::sync::Arc;
use crate::error::OmniResult;
use crate::unit::UnitId;

/// 寻址模式
#[derive(Debug, Clone, Copy)]
pub enum AddrMode {
    /// 各节点独立地址空间，通过映射窗口互访（PCIe P2P / NVLink 风格）
    Windowed,
    /// 所有节点共享统一物理地址空间（NUMA / CXL.mem / 片上 NoC）
    Unified,
}

/// 节点类型
#[derive(Debug, Clone, Copy)]
pub enum NodeKind {
    CpuSocket,
    Accelerator,
    MemoryDevice,
}

pub struct InterconnectNode {
    pub unit_id:         UnitId,
    pub node_kind:       NodeKind,
    pub base_addr:       u64,
    pub size:            u64,
    pub accessible_from: Option<Vec<UnitId>>,
}

#[derive(Debug, Clone)]
pub struct InterconnectPerf {
    pub bandwidth_gbps: f64,
    pub latency_ns:     u64,
}

/// 原子操作类型
#[derive(Debug, Clone, Copy)]
pub enum AtomicOp { Add, CmpSwap, FetchOr, FetchAnd }

/// 协议后端接口
pub trait InterconnectBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn read(&self, src: &UnitId, dst: &UnitId, addr: u64, buf: &mut [u8]) -> OmniResult<()>;
    fn write(&self, src: &UnitId, dst: &UnitId, addr: u64, buf: &[u8]) -> OmniResult<()>;
    fn atomic(&self, src: &UnitId, dst: &UnitId, addr: u64, op: AtomicOp) -> OmniResult<u64>;
}

/// PCIe P2P 后端
pub struct PciePeerBackend;
impl InterconnectBackend for PciePeerBackend {
    fn name(&self) -> &'static str { "pcie-peer" }
    fn read(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &mut [u8]) -> OmniResult<()> { Ok(()) }
    fn write(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &[u8]) -> OmniResult<()> { Ok(()) }
    fn atomic(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _op: AtomicOp) -> OmniResult<u64> { Ok(0) }
}

/// 高速直连后端（NVLink / Infinity Fabric / 片上 NoC）
pub struct DirectFabricBackend;
impl InterconnectBackend for DirectFabricBackend {
    fn name(&self) -> &'static str { "direct-fabric" }
    fn read(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &mut [u8]) -> OmniResult<()> { Ok(()) }
    fn write(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &[u8]) -> OmniResult<()> { Ok(()) }
    fn atomic(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _op: AtomicOp) -> OmniResult<u64> { Ok(0) }
}

pub struct InterconnectBus {
    pub id:        String,
    pub addr_mode: AddrMode,
    pub nodes:     Vec<InterconnectNode>,
    pub perf:      Option<InterconnectPerf>,
    pub backend:   Box<dyn InterconnectBackend>,
}

impl InterconnectBus {
    pub fn new_windowed(id: impl Into<String>) -> Self {
        Self { id: id.into(), addr_mode: AddrMode::Windowed, nodes: Vec::new(),
               perf: None, backend: Box::new(PciePeerBackend) }
    }

    pub fn new_unified(id: impl Into<String>) -> Self {
        Self { id: id.into(), addr_mode: AddrMode::Unified, nodes: Vec::new(),
               perf: None, backend: Box::new(DirectFabricBackend) }
    }
}
