//! GpgpuVram — VRAM backing store, RwLock<Vec<u8>> backed MemTarget.

use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use std::sync::RwLock;

pub struct GpgpuVram {
    data: RwLock<Vec<u8>>,
}

impl GpgpuVram {
    pub fn new(size: usize) -> Self {
        Self {
            data: RwLock::new(vec![0u8; size]),
        }
    }

    pub fn read_u32(&self, offset: u64) -> OmniResult<u32> {
        self.read(offset, MemAccessSize::U32).map(|v| v as u32)
    }

    pub fn write_u32(&self, offset: u64, val: u32) -> OmniResult<()> {
        self.write(offset, MemAccessSize::U32, val as u64)
    }
}

impl MemTarget for GpgpuVram {
    fn target_id(&self) -> u32 {
        0
    }
    fn base(&self) -> u64 {
        0
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
            return Err(OmniError::AddressNotMapped(offset));
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
            return Err(OmniError::AddressNotMapped(offset));
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
            return Err(OmniError::AddressNotMapped(offset));
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
            return Err(OmniError::AddressNotMapped(offset));
        }
        data[off..off + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}
