//! PCIe bus — ECAM config space, BDF routing, BAR mapping, root complex
//!
//! Type 0 config header layout (256 bytes):
//!   0x00  vendor_id (u16) + device_id (u16)
//!   0x04  command (u16) + status (u16)
//!   0x08  revision_id (u8) + class_code (u24)
//!   0x0C  cache_line_size (u8) + latency_timer (u8) + header_type (u8) + bist (u8)
//!   0x10–0x24  BAR0–BAR5 (6 × u32)
//!
//! ECAM address decoding:
//!   addr = ecam_base + (bus << 20 | device << 15 | function << 12 | offset)

pub fn register() {}

use std::sync::{Arc, RwLock};
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::mem::{MemTarget, MemAttrs, MemAccessSize};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};

const CONFIG_SPACE_SIZE: usize = 256;
const BAR_COUNT: usize = 6;

const CFG_VENDOR_ID: usize    = 0x00;
const CFG_DEVICE_ID: usize    = 0x02;
const CFG_COMMAND: usize      = 0x04;
const CFG_STATUS: usize       = 0x06;
const CFG_REVISION: usize     = 0x08;
const CFG_CLASS_CODE: usize   = 0x09;
const CFG_HEADER_TYPE: usize  = 0x0E;
const CFG_BAR0: usize         = 0x10;
const CFG_SUBSYS_VENDOR: usize = 0x2C;
const CFG_SUBSYS_ID: usize    = 0x2E;
const CFG_INT_LINE: usize     = 0x3C;
const CFG_INT_PIN: usize      = 0x3D;

pub struct PcieConfigSpace {
    data: [u8; CONFIG_SPACE_SIZE],
}

impl PcieConfigSpace {
    pub fn new(vendor_id: u16, device_id: u16, class_code: u32) -> Self {
        let mut data = [0u8; CONFIG_SPACE_SIZE];
        data[CFG_VENDOR_ID..CFG_VENDOR_ID + 2].copy_from_slice(&vendor_id.to_le_bytes());
        data[CFG_DEVICE_ID..CFG_DEVICE_ID + 2].copy_from_slice(&device_id.to_le_bytes());
        let cc_bytes = class_code.to_le_bytes();
        data[CFG_CLASS_CODE] = cc_bytes[0];
        data[CFG_CLASS_CODE + 1] = cc_bytes[1];
        data[CFG_CLASS_CODE + 2] = cc_bytes[2];
        data[CFG_HEADER_TYPE] = 0x00;
        // Status: capabilities list not present
        data[CFG_STATUS] = 0x00;
        data[CFG_STATUS + 1] = 0x00;
        Self { data }
    }

    pub fn read_u8(&self, offset: usize) -> u8 {
        if offset < CONFIG_SPACE_SIZE { self.data[offset] } else { 0xFF }
    }

    pub fn read_u16(&self, offset: usize) -> u16 {
        if offset + 1 < CONFIG_SPACE_SIZE {
            u16::from_le_bytes([self.data[offset], self.data[offset + 1]])
        } else {
            0xFFFF
        }
    }

    pub fn read_u32(&self, offset: usize) -> u32 {
        if offset + 3 < CONFIG_SPACE_SIZE {
            u32::from_le_bytes([
                self.data[offset], self.data[offset + 1],
                self.data[offset + 2], self.data[offset + 3],
            ])
        } else {
            0xFFFF_FFFF
        }
    }

    pub fn write_u8(&mut self, offset: usize, val: u8) {
        if offset < CONFIG_SPACE_SIZE && self.is_writable(offset) {
            self.data[offset] = val;
        }
    }

    pub fn write_u16(&mut self, offset: usize, val: u16) {
        if offset + 1 < CONFIG_SPACE_SIZE && self.is_writable(offset) {
            self.data[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
        }
    }

    pub fn write_u32(&mut self, offset: usize, val: u32) {
        if offset + 3 < CONFIG_SPACE_SIZE && self.is_writable(offset) {
            self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        }
    }

    pub fn set_bar(&mut self, index: usize, val: u32) {
        if index < BAR_COUNT {
            let off = CFG_BAR0 + index * 4;
            self.data[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }
    }

    pub fn bar(&self, index: usize) -> u32 {
        if index < BAR_COUNT {
            self.read_u32(CFG_BAR0 + index * 4)
        } else {
            0
        }
    }

    fn is_writable(&self, offset: usize) -> bool {
        !matches!(offset,
            CFG_VENDOR_ID | 0x01 |
            CFG_DEVICE_ID | 0x03 |
            CFG_REVISION |
            CFG_CLASS_CODE | 0x0A | 0x0B |
            CFG_HEADER_TYPE
        )
    }
}

/// A PCIe BAR: maps a device memory region into the system address space.
pub struct PcieBar {
    pub index: u8,
    pub base: u64,
    pub size: u64,
    pub target: Arc<dyn MemTarget>,
}

/// PCIe device descriptor with BDF address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bdf {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl Bdf {
    pub fn new(bus: u8, device: u8, function: u8) -> Self {
        Self { bus, device, function }
    }

    pub fn ecam_offset(&self) -> u64 {
        ((self.bus as u64) << 20) | ((self.device as u64) << 15) | ((self.function as u64) << 12)
    }
}

impl std::fmt::Display for Bdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// Trait for PCIe endpoint devices.
pub trait PcieDevice: Send + Sync {
    fn vendor_id(&self) -> u16;
    fn device_id(&self) -> u16;
    fn class_code(&self) -> u32;
    fn bdf(&self) -> Bdf;
    fn bars(&self) -> &[PcieBar];
    fn config_read(&self, offset: usize) -> u32;
    fn config_write(&mut self, offset: usize, val: u32);
}

/// A PCIe endpoint wrapping MemTargets as BARs.
pub struct PcieEndpoint {
    bdf: Bdf,
    config: PcieConfigSpace,
    bars: Vec<PcieBar>,
}

impl PcieEndpoint {
    pub fn new(bdf: Bdf, vendor_id: u16, device_id: u16, class_code: u32) -> Self {
        let config = PcieConfigSpace::new(vendor_id, device_id, class_code);
        Self { bdf, config, bars: Vec::new() }
    }

    pub fn add_bar(&mut self, index: u8, base: u64, size: u64, target: Arc<dyn MemTarget>) {
        self.config.set_bar(index as usize, base as u32);
        self.bars.push(PcieBar { index, base, size, target });
    }
}

impl PcieDevice for PcieEndpoint {
    fn vendor_id(&self) -> u16 { self.config.read_u16(CFG_VENDOR_ID) }
    fn device_id(&self) -> u16 { self.config.read_u16(CFG_DEVICE_ID) }
    fn class_code(&self) -> u32 {
        let b0 = self.config.read_u8(CFG_CLASS_CODE) as u32;
        let b1 = self.config.read_u8(CFG_CLASS_CODE + 1) as u32;
        let b2 = self.config.read_u8(CFG_CLASS_CODE + 2) as u32;
        b0 | (b1 << 8) | (b2 << 16)
    }
    fn bdf(&self) -> Bdf { self.bdf }
    fn bars(&self) -> &[PcieBar] { &self.bars }
    fn config_read(&self, offset: usize) -> u32 { self.config.read_u32(offset & 0xFC) }
    fn config_write(&mut self, offset: usize, val: u32) { self.config.write_u32(offset & 0xFC, val); }
}

/// PCIe Root Complex — holds devices, routes ECAM config and BAR memory transactions.
///
/// ECAM layout: ecam_base + (bus << 20 | device << 15 | function << 12 | reg_offset)
/// Each function gets a 4 KiB config window.
pub struct PcieRootComplex {
    id: UnitId,
    ecam_base: u64,
    ecam_size: u64,
    devices: RwLock<Vec<Box<dyn PcieDevice>>>,
}

impl PcieRootComplex {
    pub fn new(id: &str, ecam_base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            ecam_base,
            ecam_size: 0x0100_0000, // 256 buses × 32 devices × 8 functions × 4 KiB = 256 MiB
            devices: RwLock::new(Vec::new()),
        }
    }

    pub fn with_ecam_size(mut self, size: u64) -> Self {
        self.ecam_size = size;
        self
    }

    pub fn attach(&self, dev: Box<dyn PcieDevice>) {
        self.devices.write().unwrap().push(dev);
    }

    pub fn device_count(&self) -> usize {
        self.devices.read().unwrap().len()
    }

    pub fn map_bars(&self, bus: &mut MemBus) -> OmniResult<()> {
        let devs = self.devices.read().unwrap();
        for dev in devs.iter() {
            for bar in dev.bars() {
                bus.map(bar.target.clone(), bar.base, MemAttrs::rw(), None)?;
            }
        }
        Ok(())
    }

    pub fn config_read_bdf(&self, bdf: Bdf, offset: usize) -> u32 {
        let devs = self.devices.read().unwrap();
        devs.iter()
            .find(|d| d.bdf() == bdf)
            .map(|d| d.config_read(offset))
            .unwrap_or(0xFFFF_FFFF)
    }

    pub fn config_write_bdf(&self, bdf: Bdf, offset: usize, val: u32) {
        let mut devs = self.devices.write().unwrap();
        if let Some(dev) = devs.iter_mut().find(|d| d.bdf() == bdf) {
            dev.config_write(offset, val);
        }
    }

    fn decode_ecam(&self, offset: u64) -> (Bdf, usize) {
        let bus = ((offset >> 20) & 0xFF) as u8;
        let device = ((offset >> 15) & 0x1F) as u8;
        let function = ((offset >> 12) & 0x07) as u8;
        let reg = (offset & 0xFFF) as usize;
        (Bdf::new(bus, device, function), reg)
    }
}

impl Unit for PcieRootComplex {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "pcie-rc" }
    fn caps(&self) -> UnitCaps { UnitCaps::ADDRESSABLE }
    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }
    fn reset(&mut self) -> OmniResult<()> { Ok(()) }
}

impl MemTarget for PcieRootComplex {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { self.ecam_base }
    fn size(&self) -> u64 { self.ecam_size }

    fn read(&self, offset: u64, size: MemAccessSize) -> OmniResult<u64> {
        let (bdf, reg) = self.decode_ecam(offset);
        let devs = self.devices.read().map_err(|e| OmniError::Internal(e.to_string()))?;
        let val = devs.iter()
            .find(|d| d.bdf() == bdf)
            .map(|d| {
                match size {
                    MemAccessSize::U8 => {
                        let dword = d.config_read(reg & 0xFC);
                        let shift = (reg & 3) * 8;
                        ((dword >> shift) & 0xFF) as u64
                    }
                    MemAccessSize::U16 => {
                        let dword = d.config_read(reg & 0xFC);
                        let shift = (reg & 2) * 8;
                        ((dword >> shift) & 0xFFFF) as u64
                    }
                    _ => d.config_read(reg) as u64,
                }
            })
            .unwrap_or(match size {
                MemAccessSize::U8 => 0xFF,
                MemAccessSize::U16 => 0xFFFF,
                _ => 0xFFFF_FFFF,
            });
        Ok(val)
    }

    fn write(&self, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()> {
        let (bdf, reg) = self.decode_ecam(offset);
        let mut devs = self.devices.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        if let Some(dev) = devs.iter_mut().find(|d| d.bdf() == bdf) {
            match size {
                MemAccessSize::U8 => {
                    let cur = dev.config_read(reg & 0xFC);
                    let shift = (reg & 3) * 8;
                    let mask = !(0xFF << shift);
                    dev.config_write(reg & 0xFC, (cur & mask) | ((val as u32 & 0xFF) << shift));
                }
                MemAccessSize::U16 => {
                    let cur = dev.config_read(reg & 0xFC);
                    let shift = (reg & 2) * 8;
                    let mask = !(0xFFFF << shift);
                    dev.config_write(reg & 0xFC, (cur & mask) | ((val as u32 & 0xFFFF) << shift));
                }
                _ => {
                    dev.config_write(reg, val as u32);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ep(bdf: Bdf, vendor: u16, device: u16, bar_base: u64) -> PcieEndpoint {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let mut ep = PcieEndpoint::new(bdf, vendor, device, 0x030000);
        ep.add_bar(0, bar_base, 0x1000, dram);
        ep
    }

    #[test]
    fn test_bdf_display() {
        let bdf = Bdf::new(0, 1, 0);
        assert_eq!(format!("{bdf}"), "00:01.0");
    }

    #[test]
    fn test_bdf_ecam_offset() {
        let bdf = Bdf::new(0, 2, 1);
        // bus=0, device=2 (<<15 = 0x10000), function=1 (<<12 = 0x1000)
        assert_eq!(bdf.ecam_offset(), 0x0001_1000);
    }

    #[test]
    fn test_config_space_identity() {
        let cfg = PcieConfigSpace::new(0x1234, 0x5678, 0x030000);
        assert_eq!(cfg.read_u16(CFG_VENDOR_ID), 0x1234);
        assert_eq!(cfg.read_u16(CFG_DEVICE_ID), 0x5678);
        assert_eq!(cfg.read_u8(CFG_HEADER_TYPE), 0x00);
        assert_eq!(cfg.read_u8(0x0B), 0x03);
    }

    #[test]
    fn test_config_space_bar_readback() {
        let mut cfg = PcieConfigSpace::new(0xAAAA, 0xBBBB, 0);
        cfg.set_bar(0, 0xF000_0000);
        cfg.set_bar(2, 0xE000_0000);
        assert_eq!(cfg.bar(0), 0xF000_0000);
        assert_eq!(cfg.bar(1), 0);
        assert_eq!(cfg.bar(2), 0xE000_0000);
    }

    #[test]
    fn test_config_space_readonly_fields() {
        let mut cfg = PcieConfigSpace::new(0x1234, 0x5678, 0x030000);
        cfg.write_u16(CFG_VENDOR_ID, 0xFFFF);
        assert_eq!(cfg.read_u16(CFG_VENDOR_ID), 0x1234);
        cfg.write_u16(CFG_COMMAND, 0x0007);
        assert_eq!(cfg.read_u16(CFG_COMMAND), 0x0007);
    }

    #[test]
    fn test_endpoint_bdf() {
        let ep = make_ep(Bdf::new(0, 3, 0), 0x10DE, 0x2204, 0xC000_0000);
        assert_eq!(ep.bdf(), Bdf::new(0, 3, 0));
        assert_eq!(ep.vendor_id(), 0x10DE);
        assert_eq!(ep.device_id(), 0x2204);
        assert_eq!(ep.bars().len(), 1);
        assert_eq!(ep.bars()[0].base, 0xC000_0000);
    }

    #[test]
    fn test_endpoint_multiple_bars() {
        let bdf = Bdf::new(0, 0, 0);
        let mut ep = PcieEndpoint::new(bdf, 0x1234, 0x5678, 0);
        let dram0 = Arc::new(rhetsim_mem_dram::Dram::with_base("bar0", 0x1000, 0));
        let dram1 = Arc::new(rhetsim_mem_dram::Dram::with_base("bar1", 0x4000, 0));
        ep.add_bar(0, 0xA000_0000, 0x1000, dram0);
        ep.add_bar(2, 0xB000_0000, 0x4000, dram1);
        assert_eq!(ep.bars().len(), 2);
        assert_eq!(ep.config_read(CFG_BAR0), 0xA000_0000);
        assert_eq!(ep.config_read(CFG_BAR0 + 8), 0xB000_0000); // BAR2
    }

    #[test]
    fn test_rc_ecam_config_read() {
        let rc = PcieRootComplex::new("pcie0", 0x3000_0000);
        let ep = make_ep(Bdf::new(0, 1, 0), 0xABCD, 0x1234, 0xD000_0000);
        rc.attach(Box::new(ep));

        // ECAM offset for bus=0, device=1, function=0: (1 << 15) = 0x8000
        let vendor = rc.read(0x8000, MemAccessSize::U16).unwrap();
        assert_eq!(vendor, 0xABCD);
        let device = rc.read(0x8002, MemAccessSize::U16).unwrap();
        assert_eq!(device, 0x1234);
        // Full dword at offset 0
        let id = rc.read(0x8000, MemAccessSize::U32).unwrap();
        assert_eq!(id & 0xFFFF, 0xABCD);
        assert_eq!(id >> 16, 0x1234);
    }

    #[test]
    fn test_rc_ecam_config_write() {
        let rc = PcieRootComplex::new("pcie0", 0x3000_0000);
        let ep = make_ep(Bdf::new(0, 0, 0), 0xBEEF, 0xCAFE, 0xD000_0000);
        rc.attach(Box::new(ep));

        // Write command register via ECAM MMIO
        rc.write(CFG_COMMAND as u64, MemAccessSize::U16, 0x0006).unwrap();
        let cmd = rc.read(CFG_COMMAND as u64, MemAccessSize::U16).unwrap();
        assert_eq!(cmd, 0x0006);

        // Verify vendor_id is still read-only
        rc.write(CFG_VENDOR_ID as u64, MemAccessSize::U16, 0xFFFF).unwrap();
        let vendor = rc.read(CFG_VENDOR_ID as u64, MemAccessSize::U16).unwrap();
        assert_eq!(vendor, 0xBEEF);
    }

    #[test]
    fn test_rc_nonexistent_device() {
        let rc = PcieRootComplex::new("pcie0", 0x3000_0000);
        // No devices attached — should return 0xFFFF_FFFF
        let val = rc.read(0x0000, MemAccessSize::U32).unwrap();
        assert_eq!(val, 0xFFFF_FFFF);
        let val16 = rc.read(0x0000, MemAccessSize::U16).unwrap();
        assert_eq!(val16, 0xFFFF);
    }

    #[test]
    fn test_rc_bar_mapping_to_membus() {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let mut ep = PcieEndpoint::new(Bdf::new(0, 0, 0), 0x10DE, 0x2204, 0x030200);
        ep.add_bar(0, 0xC000_0000, 0x1000, dram.clone() as Arc<dyn MemTarget>);

        let rc = PcieRootComplex::new("pcie0", 0xFE00_0000);
        rc.attach(Box::new(ep));

        let mut bus = MemBus::new("sys-bus");
        rc.map_bars(&mut bus).unwrap();
        let bus = Arc::new(bus);

        bus.write(0xC000_0000, MemAccessSize::U32, 0xDEAD_BEEF, 0).unwrap();
        let val = bus.read(0xC000_0000, MemAccessSize::U32, 0).unwrap();
        assert_eq!(val, 0xDEAD_BEEF);
        assert_eq!(dram.read(0, MemAccessSize::U32).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn test_multiple_devices_bdf() {
        let rc = PcieRootComplex::new("pcie0", 0x3000_0000);

        let ep1 = make_ep(Bdf::new(0, 0, 0), 0x1111, 0x2222, 0xA000_0000);
        let ep2 = make_ep(Bdf::new(0, 1, 0), 0x3333, 0x4444, 0xB000_0000);
        rc.attach(Box::new(ep1));
        rc.attach(Box::new(ep2));

        // Device 0 at ECAM offset 0
        let id0 = rc.read(0x0000, MemAccessSize::U32).unwrap();
        assert_eq!(id0 & 0xFFFF, 0x1111);

        // Device 1 at ECAM offset 0x8000 (device=1 << 15)
        let id1 = rc.read(0x8000, MemAccessSize::U32).unwrap();
        assert_eq!(id1 & 0xFFFF, 0x3333);

        // BDF-based API
        assert_eq!(rc.config_read_bdf(Bdf::new(0, 0, 0), 0) & 0xFFFF, 0x1111);
        assert_eq!(rc.config_read_bdf(Bdf::new(0, 1, 0), 0) & 0xFFFF, 0x3333);
    }

    #[test]
    fn test_ecam_u8_read() {
        let rc = PcieRootComplex::new("pcie0", 0x3000_0000);
        let ep = make_ep(Bdf::new(0, 0, 0), 0xAB_CD, 0x12_34, 0xD000_0000);
        rc.attach(Box::new(ep));

        // Byte 0 of vendor_id (little-endian)
        let b0 = rc.read(0x00, MemAccessSize::U8).unwrap();
        assert_eq!(b0, 0xCD);
        let b1 = rc.read(0x01, MemAccessSize::U8).unwrap();
        assert_eq!(b1, 0xAB);
        // Byte 2 = device_id low
        let b2 = rc.read(0x02, MemAccessSize::U8).unwrap();
        assert_eq!(b2, 0x34);
        let b3 = rc.read(0x03, MemAccessSize::U8).unwrap();
        assert_eq!(b3, 0x12);
    }

    #[test]
    fn test_config_write_bdf() {
        let rc = PcieRootComplex::new("pcie0", 0x3000_0000);
        let ep = make_ep(Bdf::new(0, 2, 0), 0x1234, 0x5678, 0xC000_0000);
        rc.attach(Box::new(ep));

        rc.config_write_bdf(Bdf::new(0, 2, 0), CFG_COMMAND, 0x0007);
        let cmd = rc.config_read_bdf(Bdf::new(0, 2, 0), CFG_COMMAND);
        assert_eq!(cmd & 0xFFFF, 0x0007);
    }
}
