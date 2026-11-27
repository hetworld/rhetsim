//! PcieBus / PcieEndpoint / PcieRootComplex — PCIe 总线

use crate::error::OmniResult;
use crate::irq::MsiSender;
use crate::mem::MemAccessSize;
use crate::unit::UnitId;
use std::sync::Arc;

/// BAR 描述
#[derive(Debug, Clone)]
pub struct BarDesc {
    /// BAR 索引
    pub index: u8,
    /// BAR 大小
    pub size: u64,
    /// 宿主侧映射基址
    pub host_base: u64,
    /// BAR 类型
    pub kind: BarKind,
}

/// BAR 类型
#[derive(Debug, Clone, Copy, Default)]
pub enum BarKind {
    /// 32 位内存 BAR
    #[default]
    Memory32,
    /// 64 位内存 BAR
    Memory64,
    /// I/O BAR
    Io,
}

/// PCIe Endpoint 描述
pub struct PcieEndpointDesc {
    /// 对应 Unit
    pub unit: UnitId,
    /// PCI 总线号
    pub bus: u8,
    /// 设备号
    pub device: u8,
    /// 功能号
    pub function: u8,
    /// BAR 列表
    pub bars: Vec<BarDesc>,
}

/// PCIe Endpoint — 实现此 trait 的 Unit 可作为 PCIe 设备
pub trait PcieEndpoint: Send + Sync {
    /// 返回 BDF（bus, device, function）
    fn bdf(&self) -> (u8, u8, u8);
    /// 返回 BAR 列表
    fn bars(&self) -> &[BarDesc];
    /// 读取配置空间
    fn config_read(&self, offset: u16, size: MemAccessSize) -> OmniResult<u32>;
    /// 写入配置空间
    fn config_write(&self, offset: u16, size: MemAccessSize, val: u32) -> OmniResult<()>;
    /// 返回 MSI 发送器（若支持）
    fn msi_sender(&self) -> Option<Arc<dyn MsiSender>> {
        None
    }

    /// EP 反向 DMA：EP 主动访问 Host 内存（DPU 场景）
    fn dma_read_host(&self, host_addr: u64, buf: &mut [u8]) -> OmniResult<()>;
    /// EP 反向 DMA：EP 主动写 Host 内存
    fn dma_write_host(&self, host_addr: u64, buf: &[u8]) -> OmniResult<()>;
}

/// PCIe Root Complex
pub trait PcieRootComplex: Send + Sync {
    /// 读取 Host 内存
    fn host_mem_read(&self, addr: u64, buf: &mut [u8]) -> OmniResult<()>;
    /// 写入 Host 内存
    fn host_mem_write(&self, addr: u64, buf: &[u8]) -> OmniResult<()>;
    /// 分发 MSI
    fn dispatch_msi(&self, addr: u64, data: u32) -> OmniResult<()>;
}

/// PCIe 总线配置
pub struct PcieBusConfig {
    /// ECAM 基址
    pub ecam_base: u64,
    /// ECAM 大小
    pub ecam_size: u64,
    /// MMIO 基址
    pub mmio_base: u64,
    /// MMIO 大小
    pub mmio_size: u64,
}

impl Default for PcieBusConfig {
    fn default() -> Self {
        Self {
            ecam_base: 0x3000_0000,
            ecam_size: 0x1000_0000,
            mmio_base: 0x4000_0000,
            mmio_size: 0x4000_0000,
        }
    }
}

/// PCIe 总线
pub struct PcieBus {
    /// 总线 ID
    pub id: String,
    /// Root Complex
    pub root_complex: Option<Arc<dyn PcieRootComplex>>,
    /// 已挂载的 Endpoint
    pub endpoints: Vec<Arc<dyn PcieEndpoint>>,
    /// 总线配置
    pub config: PcieBusConfig,
}

impl PcieBus {
    /// 创建空的 PCIe 总线
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root_complex: None,
            endpoints: Vec::new(),
            config: PcieBusConfig::default(),
        }
    }

    /// 挂载一个 Endpoint
    pub fn add_endpoint(&mut self, ep: Arc<dyn PcieEndpoint>) {
        self.endpoints.push(ep);
    }
}
