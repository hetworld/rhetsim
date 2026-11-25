//! GpgpuCtrl — control registers (MemTarget) and command processing logic.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use rhetsim_core::error::OmniResult;
use rhetsim_core::mem::{MemTarget, MemAccessSize};
use crate::vram_ctrl::GpgpuVram;

const CMD_DOORBELL: u64 = 0x00;
const CMD_STATUS: u64   = 0x04;
const CMD_OP: u64       = 0x08;
const CMD_LEN: u64      = 0x0C;
const CMD_SRC_A: u64    = 0x10;
const CMD_SRC_B: u64    = 0x14;
const CMD_DST: u64      = 0x18;

pub const STATUS_IDLE: u32 = 0;
pub const STATUS_BUSY: u32 = 1;
pub const STATUS_DONE: u32 = 2;

pub const OP_NOP: u32        = 0;
pub const OP_VECTOR_ADD: u32 = 1;
pub const OP_VECTOR_MUL: u32 = 2;

pub struct GpgpuCtrl {
    status: AtomicU32,
    pending: AtomicU32,
    op: AtomicU32,
    len: AtomicU32,
    src_a: AtomicU32,
    src_b: AtomicU32,
    dst: AtomicU32,
    vram: Arc<GpgpuVram>,
}

impl GpgpuCtrl {
    pub fn new(vram: Arc<GpgpuVram>) -> Self {
        Self {
            status: AtomicU32::new(STATUS_IDLE),
            pending: AtomicU32::new(0),
            op: AtomicU32::new(OP_NOP),
            len: AtomicU32::new(0),
            src_a: AtomicU32::new(0),
            src_b: AtomicU32::new(0),
            dst: AtomicU32::new(0),
            vram,
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire) != 0
    }

    pub fn process_command(&self) -> OmniResult<u64> {
        if !self.has_pending() {
            return Ok(0);
        }

        self.status.store(STATUS_BUSY, Ordering::Release);

        let op = self.op.load(Ordering::Acquire);
        let len = self.len.load(Ordering::Acquire) as usize;
        let src_a_off = self.src_a.load(Ordering::Acquire) as u64;
        let src_b_off = self.src_b.load(Ordering::Acquire) as u64;
        let dst_off = self.dst.load(Ordering::Acquire) as u64;

        match op {
            OP_VECTOR_ADD => {
                for i in 0..len {
                    let off = (i as u64) * 4;
                    let a = self.vram.read_u32(src_a_off + off)?;
                    let b = self.vram.read_u32(src_b_off + off)?;
                    self.vram.write_u32(dst_off + off, a.wrapping_add(b))?;
                }
            }
            OP_VECTOR_MUL => {
                for i in 0..len {
                    let off = (i as u64) * 4;
                    let a = self.vram.read_u32(src_a_off + off)?;
                    let b = self.vram.read_u32(src_b_off + off)?;
                    self.vram.write_u32(dst_off + off, a.wrapping_mul(b))?;
                }
            }
            _ => {}
        }

        self.pending.store(0, Ordering::Release);
        self.status.store(STATUS_DONE, Ordering::Release);

        Ok(len as u64)
    }
}

impl MemTarget for GpgpuCtrl {
    fn target_id(&self) -> u32 { 0 }
    fn base(&self) -> u64 { 0 }
    fn size(&self) -> u64 { 256 }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        let val = match offset {
            CMD_STATUS => self.status.load(Ordering::Acquire),
            CMD_OP     => self.op.load(Ordering::Acquire),
            CMD_LEN    => self.len.load(Ordering::Acquire),
            CMD_SRC_A  => self.src_a.load(Ordering::Acquire),
            CMD_SRC_B  => self.src_b.load(Ordering::Acquire),
            CMD_DST    => self.dst.load(Ordering::Acquire),
            _ => 0,
        };
        Ok(val as u64)
    }

    fn write(&self, offset: u64, _size: MemAccessSize, val: u64) -> OmniResult<()> {
        let v = val as u32;
        match offset {
            CMD_DOORBELL => {
                self.pending.store(1, Ordering::Release);
            }
            CMD_OP    => { self.op.store(v, Ordering::Release); }
            CMD_LEN   => { self.len.store(v, Ordering::Release); }
            CMD_SRC_A => { self.src_a.store(v, Ordering::Release); }
            CMD_SRC_B => { self.src_b.store(v, Ordering::Release); }
            CMD_DST   => { self.dst.store(v, Ordering::Release); }
            _ => {}
        }
        Ok(())
    }
}
