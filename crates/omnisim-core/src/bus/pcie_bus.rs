//! PcieBus / PcieEndpoint / PcieRootComplex — PCIe 总线

use std::sync::Arc;
use crate::error::OmniResult;
use crate::mem::{MemTarget, MemAttrs, MemAccessSize};
use crate::irq::MsiSender;
use crate::unit::UnitId;

#[derive(Debug, Clone)]
pub struct BarDesc {
    pub index:     u8,
    pub size:      u64,
    pub host_base: u64,
    pub kind:      BarKind,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum BarKind { #[default] Memory32, Memory64, Io }

pub struct PcieEndpointDesc {
    pub unit:     UnitId,
    pub bus:      u8,
    pub device:   u8,
    pub function: u8,
    pub bars:     Vec<BarDesc>,
}

/// PCIe Endpoint — 实现此 trait 的 Unit 可作为 PCIe 设备
pub trait PcieEndpoint: Send + Sync {
    fn bdf(&self) -> (u8, u8, u8);
    fn bars(&self) -> &[BarDesc];
    fn config_read(&self, offset: u16, size: MemAccessSize) -> OmniResult<u32>;
    fn config_write(&self, offset: u16, size: MemAccessSize, val: u32) -> OmniResult<()>;
    fn msi_sender(&self) -> Option<Arc<dyn MsiSender>> { None }

    /// EP 反向 DMA：EP 主动访问 Host 内存（DPU 场景）
    fn dma_read_host(&self, host_addr: u64, buf: &mut [u8]) -> OmniResult<()>;
    fn dma_write_host(&self, host_addr: u64, buf: &[u8]) -> OmniResult<()>;
}

/// PCIe Root Complex
pub trait PcieRootComplex: Send + Sync {
    fn host_mem_read(&self, addr: u64, buf: &mut [u8]) -> OmniResult<()>;
    fn host_mem_write(&self, addr: u64, buf: &[u8]) -> OmniResult<()>;
    fn dispatch_msi(&self, addr: u64, data: u32) -> OmniResult<()>;
}

pub struct PcieBusConfig {
    pub ecam_base: u64,
    pub ecam_size: u64,
    pub mmio_base: u64,
    pub mmio_size: u64,
}

impl Default for PcieBusConfig {
    fn default() -> Self {
        Self { ecam_base: 0x3000_0000, ecam_size: 0x1000_0000,
               mmio_base: 0x4000_0000, mmio_size: 0x4000_0000 }
    }
}

pub struct PcieBus {
    pub id:           String,
    pub root_complex: Option<Arc<dyn PcieRootComplex>>,
    pub endpoints:    Vec<Arc<dyn PcieEndpoint>>,
    pub config:       PcieBusConfig,
}

impl PcieBus {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), root_complex: None, endpoints: Vec::new(),
               config: PcieBusConfig::default() }
    }

    pub fn add_endpoint(&mut self, ep: Arc<dyn PcieEndpoint>) {
        self.endpoints.push(ep);
    }
}
