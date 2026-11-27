//! SRAM memory component — Vec<u8> backed, implements Unit + MemTarget.

use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::sync::RwLock;

pub struct Sram {
    id: UnitId,
    base: u64,
    data: RwLock<Vec<u8>>,
}

impl Sram {
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

impl Unit for Sram {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "sram"
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

impl MemTarget for Sram {
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
    fn test_sram_read_write() {
        let sram = Sram::new("test-sram", 1024);
        sram.write(0, MemAccessSize::U32, 0xCAFEBABE).unwrap();
        assert_eq!(sram.read(0, MemAccessSize::U32).unwrap(), 0xCAFEBABE);
    }

    #[test]
    fn test_sram_bulk() {
        let sram = Sram::new("test-sram", 1024);
        let src = b"SRAM test";
        sram.write_bulk(0, src).unwrap();
        let mut dst = vec![0u8; src.len()];
        sram.read_bulk(0, &mut dst).unwrap();
        assert_eq!(&dst, src);
    }
}
