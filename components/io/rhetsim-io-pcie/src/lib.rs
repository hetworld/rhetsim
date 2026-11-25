//! PCIe bus — config space, BAR mapping, root complex, endpoint
//!
//! Type 0 config header layout (256 bytes):
//!   0x00  vendor_id (u16) + device_id (u16)
//!   0x04  command (u16) + status (u16)
//!   0x08  revision_id (u8) + class_code (u24)
//!   0x0C  cache_line_size (u8) + latency_timer (u8) + header_type (u8) + bist (u8)
//!   0x10–0x24  BAR0–BAR5 (6 × u32)

pub fn register() {}

use std::sync::Arc;
use rhetsim_core::bus::mem_bus::MemBus;
use rhetsim_core::error::OmniResult;
use rhetsim_core::mem::{MemTarget, MemAttrs, MemAccessSize};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};

const CONFIG_SPACE_SIZE: usize = 256;
const BAR_COUNT: usize = 6;

// Config space offsets
const CFG_VENDOR_ID: usize    = 0x00;
const CFG_DEVICE_ID: usize    = 0x02;
const CFG_COMMAND: usize      = 0x04;
const CFG_STATUS: usize       = 0x06;
const CFG_REVISION: usize     = 0x08;
const CFG_CLASS_CODE: usize   = 0x09; // 3 bytes: prog_if, subclass, class
const CFG_HEADER_TYPE: usize  = 0x0E;
const CFG_BAR0: usize         = 0x10;

/// Type 0 PCIe configuration space header.
pub struct PcieConfigSpace {
    data: [u8; CONFIG_SPACE_SIZE],
}

impl PcieConfigSpace {
    pub fn new(vendor_id: u16, device_id: u16, class_code: u32) -> Self {
        let mut data = [0u8; CONFIG_SPACE_SIZE];
        // Vendor ID
        data[CFG_VENDOR_ID..CFG_VENDOR_ID + 2].copy_from_slice(&vendor_id.to_le_bytes());
        // Device ID
        data[CFG_DEVICE_ID..CFG_DEVICE_ID + 2].copy_from_slice(&device_id.to_le_bytes());
        // Class code: offset 0x09 = prog_if, 0x0A = subclass, 0x0B = class
        let cc_bytes = class_code.to_le_bytes();
        data[CFG_CLASS_CODE] = cc_bytes[0];     // prog_if
        data[CFG_CLASS_CODE + 1] = cc_bytes[1]; // subclass
        data[CFG_CLASS_CODE + 2] = cc_bytes[2]; // class
        // Header type 0
        data[CFG_HEADER_TYPE] = 0x00;
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
        // Read-only: vendor_id, device_id, revision, class_code, header_type
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
    pub base: u64,
    pub size: u64,
    pub target: Arc<dyn MemTarget>,
}

/// Trait for PCIe endpoint devices.
pub trait PcieDevice: Send + Sync {
    fn vendor_id(&self) -> u16;
    fn device_id(&self) -> u16;
    fn class_code(&self) -> u32;
    fn bars(&self) -> &[PcieBar];
    fn config(&self) -> &PcieConfigSpace;
    fn config_mut(&mut self) -> &mut PcieConfigSpace;
}

/// A simple PCIe endpoint wrapping a single MemTarget as BAR0.
pub struct PcieEndpoint {
    config: PcieConfigSpace,
    bars: Vec<PcieBar>,
    vendor: u16,
    device: u16,
    class: u32,
}

impl PcieEndpoint {
    pub fn new(
        vendor_id: u16,
        device_id: u16,
        class_code: u32,
        bar_target: Arc<dyn MemTarget>,
        bar_base: u64,
        bar_size: u64,
    ) -> Self {
        let mut config = PcieConfigSpace::new(vendor_id, device_id, class_code);
        config.set_bar(0, bar_base as u32);
        Self {
            config,
            bars: vec![PcieBar { base: bar_base, size: bar_size, target: bar_target }],
            vendor: vendor_id,
            device: device_id,
            class: class_code,
        }
    }
}

impl PcieDevice for PcieEndpoint {
    fn vendor_id(&self) -> u16 { self.vendor }
    fn device_id(&self) -> u16 { self.device }
    fn class_code(&self) -> u32 { self.class }
    fn bars(&self) -> &[PcieBar] { &self.bars }
    fn config(&self) -> &PcieConfigSpace { &self.config }
    fn config_mut(&mut self) -> &mut PcieConfigSpace { &mut self.config }
}

/// PCIe Root Complex — holds devices and routes config/memory transactions.
///
/// Config space MMIO: each device gets a 4 KiB window.
/// Layout: device_index * 4096 + register_offset
pub struct PcieRootComplex {
    id: UnitId,
    base: u64,
    devices: Vec<Box<dyn PcieDevice>>,
}

impl PcieRootComplex {
    pub fn new(id: &str, base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            base,
            devices: Vec::new(),
        }
    }

    pub fn attach(&mut self, dev: Box<dyn PcieDevice>) {
        self.devices.push(dev);
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Map all device BARs into the given MemBus.
    pub fn map_bars(&self, bus: &mut MemBus) -> OmniResult<()> {
        for dev in &self.devices {
            for bar in dev.bars() {
                bus.map(
                    bar.target.clone(),
                    bar.base,
                    MemAttrs::rw(),
                    None,
                )?;
            }
        }
        Ok(())
    }

    pub fn config_read(&self, device_idx: usize, offset: u8) -> u32 {
        if let Some(dev) = self.devices.get(device_idx) {
            dev.config().read_u32(offset as usize & 0xFC)
        } else {
            0xFFFF_FFFF
        }
    }

    pub fn config_write(&mut self, device_idx: usize, offset: u8, val: u32) {
        if let Some(dev) = self.devices.get_mut(device_idx) {
            dev.config_mut().write_u32(offset as usize & 0xFC, val);
        }
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
    fn base(&self) -> u64 { self.base }

    fn size(&self) -> u64 {
        // 4 KiB per device slot, minimum 1 slot
        let slots = if self.devices.is_empty() { 1 } else { self.devices.len() };
        (slots as u64) * 4096
    }

    fn read(&self, offset: u64, size: MemAccessSize) -> OmniResult<u64> {
        let dev_idx = (offset / 4096) as usize;
        let reg_off = (offset % 4096) as usize;

        if let Some(dev) = self.devices.get(dev_idx) {
            let val = match size {
                MemAccessSize::U8 => dev.config().read_u8(reg_off) as u64,
                MemAccessSize::U16 => dev.config().read_u16(reg_off) as u64,
                _ => dev.config().read_u32(reg_off & 0xFC) as u64,
            };
            Ok(val)
        } else {
            Ok(0xFFFF_FFFF)
        }
    }

    fn write(&self, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()> {
        // Config writes need &mut self, but MemTarget::write takes &self.
        // For functional sim, we skip runtime config writes through MMIO.
        // Use config_write() directly for mutable access.
        let _ = (offset, size, val);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_space_identity() {
        let cfg = PcieConfigSpace::new(0x1234, 0x5678, 0x030000);
        assert_eq!(cfg.read_u16(CFG_VENDOR_ID), 0x1234);
        assert_eq!(cfg.read_u16(CFG_DEVICE_ID), 0x5678);
        assert_eq!(cfg.read_u8(CFG_HEADER_TYPE), 0x00);
        // class at 0x0B, subclass at 0x0A, prog_if at 0x09
        assert_eq!(cfg.read_u8(0x0B), 0x03); // display controller class
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
        // vendor_id is read-only
        cfg.write_u16(CFG_VENDOR_ID, 0xFFFF);
        assert_eq!(cfg.read_u16(CFG_VENDOR_ID), 0x1234);
        // command is writable
        cfg.write_u16(CFG_COMMAND, 0x0007);
        assert_eq!(cfg.read_u16(CFG_COMMAND), 0x0007);
    }

    #[test]
    fn test_config_space_command_writable() {
        let mut cfg = PcieConfigSpace::new(0x1234, 0x5678, 0);
        cfg.write_u16(CFG_COMMAND, 0x06); // bus master + memory space
        assert_eq!(cfg.read_u16(CFG_COMMAND), 0x06);
    }

    #[test]
    fn test_endpoint_creation() {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let ep = PcieEndpoint::new(0x10DE, 0x2204, 0x030200, dram, 0xC000_0000, 0x1000);
        assert_eq!(ep.vendor_id(), 0x10DE);
        assert_eq!(ep.device_id(), 0x2204);
        assert_eq!(ep.bars().len(), 1);
        assert_eq!(ep.bars()[0].base, 0xC000_0000);
        assert_eq!(ep.bars()[0].size, 0x1000);
        assert_eq!(ep.config().bar(0), 0xC000_0000u32);
    }

    #[test]
    fn test_root_complex_config_access() {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let ep = PcieEndpoint::new(0x10DE, 0x2204, 0x030200, dram, 0xC000_0000, 0x1000);

        let mut rc = PcieRootComplex::new("pcie0", 0xFE00_0000);
        rc.attach(Box::new(ep));

        assert_eq!(rc.device_count(), 1);
        // Read vendor/device ID via config_read
        let id = rc.config_read(0, 0x00);
        assert_eq!(id & 0xFFFF, 0x10DE);       // vendor
        assert_eq!(id >> 16, 0x2204);           // device
        // Non-existent device returns 0xFFFF_FFFF
        assert_eq!(rc.config_read(1, 0x00), 0xFFFF_FFFF);
    }

    #[test]
    fn test_root_complex_config_write() {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let ep = PcieEndpoint::new(0xBEEF, 0xCAFE, 0, dram, 0xD000_0000, 0x1000);

        let mut rc = PcieRootComplex::new("pcie0", 0xFE00_0000);
        rc.attach(Box::new(ep));

        // Write command register
        rc.config_write(0, 0x04, 0x0006);
        let cmd = rc.config_read(0, 0x04);
        assert_eq!(cmd & 0xFFFF, 0x0006);
    }

    #[test]
    fn test_bar_mapping_to_membus() {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let ep = PcieEndpoint::new(0x10DE, 0x2204, 0x030200,
                                   dram.clone() as Arc<dyn MemTarget>,
                                   0xC000_0000, 0x1000);

        let mut rc = PcieRootComplex::new("pcie0", 0xFE00_0000);
        rc.attach(Box::new(ep));

        let mut bus = MemBus::new("sys-bus");
        rc.map_bars(&mut bus).unwrap();
        let bus = Arc::new(bus);

        // Write through bus at BAR address
        bus.write(0xC000_0000, MemAccessSize::U32, 0xDEAD_BEEF, 0).unwrap();
        // Read back through bus
        let val = bus.read(0xC000_0000, MemAccessSize::U32, 0).unwrap();
        assert_eq!(val, 0xDEAD_BEEF);
        // Also visible directly on DRAM
        assert_eq!(dram.read(0, MemAccessSize::U32).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn test_rc_mmio_config_read() {
        let dram = Arc::new(rhetsim_mem_dram::Dram::with_base("vram", 0x1000, 0));
        let ep = PcieEndpoint::new(0xABCD, 0x1234, 0, dram, 0xD000_0000, 0x1000);

        let rc = PcieRootComplex::new("pcie0", 0xFE00_0000);
        let mut rc = rc;
        rc.attach(Box::new(ep));

        // Read via MemTarget interface (MMIO config space)
        // Device 0, offset 0x00 → vendor+device
        let val = rc.read(0x00, MemAccessSize::U32).unwrap();
        assert_eq!(val & 0xFFFF, 0xABCD);
        assert_eq!(val >> 16, 0x1234);

        // Read BAR0 at offset 0x10
        let bar0 = rc.read(0x10, MemAccessSize::U32).unwrap();
        assert_eq!(bar0, 0xD000_0000);
    }

    #[test]
    fn test_multiple_devices() {
        let dram1 = Arc::new(rhetsim_mem_dram::Dram::with_base("dev1", 0x1000, 0));
        let dram2 = Arc::new(rhetsim_mem_dram::Dram::with_base("dev2", 0x2000, 0));

        let ep1 = PcieEndpoint::new(0x1111, 0x2222, 0,
                                    dram1.clone() as Arc<dyn MemTarget>,
                                    0xA000_0000, 0x1000);
        let ep2 = PcieEndpoint::new(0x3333, 0x4444, 0,
                                    dram2.clone() as Arc<dyn MemTarget>,
                                    0xB000_0000, 0x2000);

        let mut rc = PcieRootComplex::new("pcie0", 0xFE00_0000);
        rc.attach(Box::new(ep1));
        rc.attach(Box::new(ep2));

        let mut bus = MemBus::new("sys-bus");
        rc.map_bars(&mut bus).unwrap();
        let bus = Arc::new(bus);

        // Write to device 1
        bus.write(0xA000_0000, MemAccessSize::U32, 0x1111_1111, 0).unwrap();
        assert_eq!(dram1.read(0, MemAccessSize::U32).unwrap(), 0x1111_1111);

        // Write to device 2
        bus.write(0xB000_0000, MemAccessSize::U32, 0x2222_2222, 0).unwrap();
        assert_eq!(dram2.read(0, MemAccessSize::U32).unwrap(), 0x2222_2222);

        // Config reads for both devices
        assert_eq!(rc.config_read(0, 0x00) & 0xFFFF, 0x1111);
        assert_eq!(rc.config_read(1, 0x00) & 0xFFFF, 0x3333);
    }
}
