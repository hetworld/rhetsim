//! InterconnectBus — 对等节点互联总线
//!
//! 覆盖：多卡 GPU 互联、NUMA 多路服务器、CXL 内存、片上 NoC。

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
    /// CPU socket
    CpuSocket,
    /// 加速器
    Accelerator,
    /// 内存设备
    MemoryDevice,
}

/// 互联节点
pub struct InterconnectNode {
    /// 节点对应 Unit
    pub unit_id: UnitId,
    /// 节点类型
    pub node_kind: NodeKind,
    /// 节点地址空间基址
    pub base_addr: u64,
    /// 节点地址空间大小
    pub size: u64,
    /// 可访问该节点的 Unit 列表；None 表示全部可见
    pub accessible_from: Option<Vec<UnitId>>,
}

/// 互联性能参数
#[derive(Debug, Clone)]
pub struct InterconnectPerf {
    /// 带宽（Gbps）
    pub bandwidth_gbps: f64,
    /// 延迟（纳秒）
    pub latency_ns: u64,
}

/// 原子操作类型
#[derive(Debug, Clone, Copy)]
pub enum AtomicOp {
    /// 加法
    Add,
    /// 比较交换
    CmpSwap,
    /// 取或
    FetchOr,
    /// 取与
    FetchAnd,
}

/// 协议后端接口
pub trait InterconnectBackend: Send + Sync {
    /// 后端名称
    fn name(&self) -> &'static str;
    /// 节点间读取
    fn read(&self, src: &UnitId, dst: &UnitId, addr: u64, buf: &mut [u8]) -> OmniResult<()>;
    /// 节点间写入
    fn write(&self, src: &UnitId, dst: &UnitId, addr: u64, buf: &[u8]) -> OmniResult<()>;
    /// 节点间原子操作
    fn atomic(&self, src: &UnitId, dst: &UnitId, addr: u64, op: AtomicOp) -> OmniResult<u64>;
}

/// PCIe P2P 后端
pub struct PciePeerBackend;
impl InterconnectBackend for PciePeerBackend {
    fn name(&self) -> &'static str {
        "pcie-peer"
    }
    fn read(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &mut [u8]) -> OmniResult<()> {
        Ok(())
    }
    fn write(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &[u8]) -> OmniResult<()> {
        Ok(())
    }
    fn atomic(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _op: AtomicOp) -> OmniResult<u64> {
        Ok(0)
    }
}

/// 高速直连后端（NVLink / Infinity Fabric / 片上 NoC）
pub struct DirectFabricBackend;
impl InterconnectBackend for DirectFabricBackend {
    fn name(&self) -> &'static str {
        "direct-fabric"
    }
    fn read(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &mut [u8]) -> OmniResult<()> {
        Ok(())
    }
    fn write(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _buf: &[u8]) -> OmniResult<()> {
        Ok(())
    }
    fn atomic(&self, _src: &UnitId, _dst: &UnitId, _addr: u64, _op: AtomicOp) -> OmniResult<u64> {
        Ok(0)
    }
}

/// 对等节点互联总线
pub struct InterconnectBus {
    /// 总线 ID
    pub id: String,
    /// 寻址模式
    pub addr_mode: AddrMode,
    /// 互联节点
    pub nodes: Vec<InterconnectNode>,
    /// 性能参数
    pub perf: Option<InterconnectPerf>,
    /// 协议后端
    pub backend: Box<dyn InterconnectBackend>,
}

impl InterconnectBus {
    /// 创建窗口式互联总线
    pub fn new_windowed(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            addr_mode: AddrMode::Windowed,
            nodes: Vec::new(),
            perf: None,
            backend: Box::new(PciePeerBackend),
        }
    }

    /// 创建统一地址空间互联总线
    pub fn new_unified(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            addr_mode: AddrMode::Unified,
            nodes: Vec::new(),
            perf: None,
            backend: Box::new(DirectFabricBackend),
        }
    }
}
