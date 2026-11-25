//! DRAM memory component — Vec<u8> backed, implements Unit + MemTarget.

use std::sync::RwLock;
use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::mem::{MemTarget, MemAccessSize};

pub struct Dram {
    id: UnitId,
    base: u64,
    data: RwLock<Vec<u8>>,
}

impl Dram {
    pub fn new(id: &str, size: usize) -> Self {
        Self {
            id: UnitId::new(id),
            base: 0,
            data: RwLock::new(vec![0u8; size]),
        }
    }

    pub fn with_base(id: &str, size: usize, base: u64) -> Self {
        Self {
            id: UnitId::new(id),
            base,
            data: RwLock::new(vec![0u8; size]),
        }
    }
}

impl Unit for Dram {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "dram" }
    fn caps(&self) -> UnitCaps { UnitCaps::ADDRESSABLE | UnitCaps::SNAPSHOTTABLE }

    fn as_mem_target(&self) -> Option<&dyn MemTarget> { Some(self) }

    fn reset(&mut self) -> OmniResult<()> {
        let mut data = self.data.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        data.fill(0);
        Ok(())
    }
}

impl MemTarget for Dram {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { self.base }

    fn size(&self) -> u64 {
        self.data.read().map(|d| d.len() as u64).unwrap_or(0)
    }

    fn read(&self, offset: u64, size: MemAccessSize) -> OmniResult<u64> {
        let data = self.data.read().map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        let len = data.len();
        let nbytes = size as usize;
        if off + nbytes > len {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        let val = match size {
            MemAccessSize::U8  => data[off] as u64,
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
        let mut data = self.data.write().map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        let len = data.len();
        let nbytes = size as usize;
        if off + nbytes > len {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        match size {
            MemAccessSize::U8  => data[off] = val as u8,
            MemAccessSize::U16 => data[off..off + 2].copy_from_slice(&(val as u16).to_le_bytes()),
            MemAccessSize::U32 => data[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes()),
            MemAccessSize::U64 => data[off..off + 8].copy_from_slice(&val.to_le_bytes()),
            MemAccessSize::U128 => data[off..off + 8].copy_from_slice(&val.to_le_bytes()),
        }
        Ok(())
    }

    fn read_bulk(&self, offset: u64, buf: &mut [u8]) -> OmniResult<()> {
        let data = self.data.read().map_err(|e| OmniError::Internal(e.to_string()))?;
        let off = offset as usize;
        if off + buf.len() > data.len() {
            return Err(OmniError::AddressNotMapped(self.base + offset));
        }
        buf.copy_from_slice(&data[off..off + buf.len()]);
        Ok(())
    }

    fn write_bulk(&self, offset: u64, buf: &[u8]) -> OmniResult<()> {
        let mut data = self.data.write().map_err(|e| OmniError::Internal(e.to_string()))?;
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
    fn test_dram_read_write_u32() {
        let dram = Dram::new("test-dram", 4096);
        dram.write(0, MemAccessSize::U32, 0xDEADBEEF).unwrap();
        let val = dram.read(0, MemAccessSize::U32).unwrap();
        assert_eq!(val, 0xDEADBEEF);
    }

    #[test]
    fn test_dram_read_write_u8() {
        let dram = Dram::new("test-dram", 4096);
        dram.write(10, MemAccessSize::U8, 0x42).unwrap();
        assert_eq!(dram.read(10, MemAccessSize::U8).unwrap(), 0x42);
    }

    #[test]
    fn test_dram_read_write_u64() {
        let dram = Dram::new("test-dram", 4096);
        dram.write(8, MemAccessSize::U64, 0x0123456789ABCDEF).unwrap();
        assert_eq!(dram.read(8, MemAccessSize::U64).unwrap(), 0x0123456789ABCDEF);
    }

    #[test]
    fn test_dram_bulk() {
        let dram = Dram::new("test-dram", 4096);
        let src = b"Hello, DRAM!";
        dram.write_bulk(100, src).unwrap();
        let mut dst = vec![0u8; src.len()];
        dram.read_bulk(100, &mut dst).unwrap();
        assert_eq!(&dst, src);
    }

    #[test]
    fn test_dram_out_of_bounds() {
        let dram = Dram::new("test-dram", 16);
        assert!(dram.write(15, MemAccessSize::U32, 0).is_err());
        assert!(dram.read(15, MemAccessSize::U32).is_err());
    }

    #[test]
    fn test_dram_reset() {
        let mut dram = Dram::new("test-dram", 4096);
        dram.write(0, MemAccessSize::U32, 0xDEADBEEF).unwrap();
        dram.reset().unwrap();
        assert_eq!(dram.read(0, MemAccessSize::U32).unwrap(), 0);
    }

    #[test]
    fn test_dram_with_base() {
        let dram = Dram::with_base("test-dram", 4096, 0x8000_0000);
        assert_eq!(dram.base(), 0x8000_0000);
        assert_eq!(dram.size(), 4096);
    }
}
