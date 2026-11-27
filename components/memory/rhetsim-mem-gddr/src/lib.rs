//! GDDR memory component — Vec<u8> backed, implements Unit + MemTarget.
//! Adds channel count and bandwidth parameters for future timing simulation.

use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::sync::RwLock;

pub struct Gddr {
    id: UnitId,
    base: u64,
    data: RwLock<Vec<u8>>,
    channels: u32,
    bandwidth_gbps: u32,
}

impl Gddr {
    pub fn new(id: &str, size: usize) -> Self {
        Self {
            id: UnitId::new(id),
            base: 0,
            data: RwLock::new(vec![0u8; size]),
            channels: 8,
            bandwidth_gbps: 448,
        }
    }

    pub fn with_base(id: &str, size: usize, base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            base,
            data: RwLock::new(vec![0u8; size]),
            channels: 8,
            bandwidth_gbps: 448,
        }
    }

    pub fn with_timing(mut self, channels: u32, bandwidth_gbps: u32) -> Self {
        self.channels = channels;
        self.bandwidth_gbps = bandwidth_gbps;
        self
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }
    pub fn bandwidth_gbps(&self) -> u32 {
        self.bandwidth_gbps
    }
}

impl Unit for Gddr {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "gddr"
    }
    fn caps(&self) -> UnitCaps {
        UnitCaps::ADDRESSABLE | UnitCaps::SNAPSHOTTABLE
    }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> {
        Some(self)
    }

    fn reset(&mut self) -> OmniResult<()> {
        let mut data = self
            .data
            .write()
            .map_err(|e| OmniError::Internal(e.to_string()))?;
        data.fill(0);
        Ok(())
    }
}

impl MemTarget for Gddr {
    fn target_id(&self) -> u32 {
        0
    }
    fn base(&self) -> u64 {
        self.base
    }

    fn size(&self) -> u64 {
        self.data.read().map(|d| d.len() as u64).unwrap_or(0)
    }

    fn read(&self, offset: u64, size: MemAccessSize) -> OmniResult<u64> {
        let data = self
            .data
            .read()
            .map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        let nbytes = size as usize;
        if off + nbytes > data.len() {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        let val = match size {
            MemAccessSize::U8 => data[off] as u64,
            MemAccessSize::U16 => {
                let mut buf = [0u8; 2];
                buf.copy_from_slice(&data[off..off + 2]);
                u16::from_le_bytes(buf) as u64
            }
            MemAccessSize::U32 => {
                let mut buf = [0u8; 4];
                buf.copy_from_slice(&data[off..off + 4]);
                u32::from_le_bytes(buf) as u64
            }
            MemAccessSize::U64 => {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&data[off..off + 8]);
                u64::from_le_bytes(buf)
            }
            MemAccessSize::U128 => {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&data[off..off + 8]);
                u64::from_le_bytes(buf)
            }
        };
        Ok(val)
    }

    fn write(&self, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()> {
        let mut data = self
            .data
            .write()
            .map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        let nbytes = size as usize;
        if off + nbytes > data.len() {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        match size {
            MemAccessSize::U8 => data[off] = val as u8,
            MemAccessSize::U16 => data[off..off + 2].copy_from_slice(&(val as u16).to_le_bytes()),
            MemAccessSize::U32 => data[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes()),
            MemAccessSize::U64 => data[off..off + 8].copy_from_slice(&val.to_le_bytes()),
            MemAccessSize::U128 => data[off..off + 8].copy_from_slice(&val.to_le_bytes()),
        }
        Ok(())
    }

    fn read_bulk(&self, offset: u64, buf: &mut [u8]) -> OmniResult<()> {
        let data = self
            .data
            .read()
            .map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        if off + buf.len() > data.len() {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        buf.copy_from_slice(&data[off..off + buf.len()]);
        Ok(())
    }

    fn write_bulk(&self, offset: u64, buf: &[u8]) -> OmniResult<()> {
        let mut data = self
            .data
            .write()
            .map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        if off + buf.len() > data.len() {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        data[off..off + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gddr_read_write_u32() {
        let gddr = Gddr::new("test-gddr", 4096);
        gddr.write(0, MemAccessSize::U32, 0xDEADBEEF).unwrap();
        assert_eq!(gddr.read(0, MemAccessSize::U32).unwrap(), 0xDEADBEEF);
    }

    #[test]
    fn test_gddr_read_write_u8() {
        let gddr = Gddr::new("test-gddr", 4096);
        gddr.write(10, MemAccessSize::U8, 0x42).unwrap();
        assert_eq!(gddr.read(10, MemAccessSize::U8).unwrap(), 0x42);
    }

    #[test]
    fn test_gddr_read_write_u64() {
        let gddr = Gddr::new("test-gddr", 4096);
        gddr.write(8, MemAccessSize::U64, 0x0123456789ABCDEF)
            .unwrap();
        assert_eq!(
            gddr.read(8, MemAccessSize::U64).unwrap(),
            0x0123456789ABCDEF
        );
    }

    #[test]
    fn test_gddr_bulk() {
        let gddr = Gddr::new("test-gddr", 4096);
        let src = b"Hello, GDDR!";
        gddr.write_bulk(100, src).unwrap();
        let mut dst = vec![0u8; src.len()];
        gddr.read_bulk(100, &mut dst).unwrap();
        assert_eq!(&dst, src);
    }

    #[test]
    fn test_gddr_out_of_bounds() {
        let gddr = Gddr::new("test-gddr", 16);
        assert!(gddr.write(15, MemAccessSize::U32, 0).is_err());
        assert!(gddr.read(15, MemAccessSize::U32).is_err());
    }

    #[test]
    fn test_gddr_reset() {
        let mut gddr = Gddr::new("test-gddr", 4096);
        gddr.write(0, MemAccessSize::U32, 0xDEADBEEF).unwrap();
        gddr.reset().unwrap();
        assert_eq!(gddr.read(0, MemAccessSize::U32).unwrap(), 0);
    }

    #[test]
    fn test_gddr_with_base() {
        let gddr = Gddr::with_base("test-gddr", 4096, 0x8000_0000);
        assert_eq!(gddr.base(), 0x8000_0000);
        assert_eq!(gddr.size(), 4096);
    }

    #[test]
    fn test_gddr_timing_params() {
        let gddr = Gddr::new("g0", 1024).with_timing(16, 896);
        assert_eq!(gddr.channels(), 16);
        assert_eq!(gddr.bandwidth_gbps(), 896);
    }

    #[test]
    fn test_gddr_default_timing() {
        let gddr = Gddr::new("g0", 1024);
        assert_eq!(gddr.channels(), 8);
        assert_eq!(gddr.bandwidth_gbps(), 448);
    }

    #[test]
    fn test_gddr_kind() {
        let gddr = Gddr::new("g0", 1024);
        assert_eq!(gddr.kind(), "gddr");
    }
}
