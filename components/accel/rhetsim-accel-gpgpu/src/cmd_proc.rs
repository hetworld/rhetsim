//! GpgpuCtrl — control registers (MemTarget) and command processing logic.

use crate::sm::{SimtHart, StreamingMultiprocessor, Warp, DEFAULT_NUM_SMS, DEFAULT_WARP_SIZE};
use crate::vram_ctrl::GpgpuVram;
use rhetsim_core::error::OmniResult;
use rhetsim_core::mem::{MemAccessSize, MemTarget};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

const CMD_DOORBELL: u64 = 0x00;
const CMD_STATUS: u64 = 0x04;
const CMD_OP: u64 = 0x08;
const CMD_LEN: u64 = 0x0C;
const CMD_SRC_A: u64 = 0x10;
const CMD_SRC_B: u64 = 0x14;
const CMD_DST: u64 = 0x18;
const CMD_KERN_ADDR: u64 = 0x1C;
const CMD_GRID_DIM: u64 = 0x20;
const CMD_BLOCK_DIM: u64 = 0x24;

pub const STATUS_IDLE: u32 = 0;
pub const STATUS_BUSY: u32 = 1;
pub const STATUS_DONE: u32 = 2;

pub const OP_NOP: u32 = 0;
pub const OP_VECTOR_ADD: u32 = 1;
pub const OP_VECTOR_MUL: u32 = 2;
pub const OP_KERNEL: u32 = 3;

pub struct GpgpuCtrl {
    status: AtomicU32,
    pending: AtomicU32,
    op: AtomicU32,
    len: AtomicU32,
    src_a: AtomicU32,
    src_b: AtomicU32,
    dst: AtomicU32,
    kern_addr: AtomicU32,
    grid_dim: AtomicU32,
    block_dim: AtomicU32,
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
            kern_addr: AtomicU32::new(0),
            grid_dim: AtomicU32::new(0),
            block_dim: AtomicU32::new(0),
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
        let result = match op {
            OP_VECTOR_ADD => self.exec_vector_op(true),
            OP_VECTOR_MUL => self.exec_vector_op(false),
            OP_KERNEL => self.exec_kernel(),
            _ => 0,
        };

        self.pending.store(0, Ordering::Release);
        self.status.store(STATUS_DONE, Ordering::Release);

        Ok(result)
    }

    fn exec_vector_op(&self, is_add: bool) -> u64 {
        let len = self.len.load(Ordering::Acquire) as usize;
        let src_a_off = self.src_a.load(Ordering::Acquire) as u64;
        let src_b_off = self.src_b.load(Ordering::Acquire) as u64;
        let dst_off = self.dst.load(Ordering::Acquire) as u64;

        for i in 0..len {
            let off = (i as u64) * 4;
            let a = self.vram.read_u32(src_a_off + off).unwrap_or(0);
            let b = self.vram.read_u32(src_b_off + off).unwrap_or(0);
            let result = if is_add {
                a.wrapping_add(b)
            } else {
                a.wrapping_mul(b)
            };
            let _ = self.vram.write_u32(dst_off + off, result);
        }
        len as u64
    }

    fn exec_kernel(&self) -> u64 {
        let kern_addr = self.kern_addr.load(Ordering::Acquire) as u64;
        let grid_dim = self.grid_dim.load(Ordering::Acquire);
        let block_dim = self.block_dim.load(Ordering::Acquire);
        let warp_size = DEFAULT_WARP_SIZE;
        let num_sms = DEFAULT_NUM_SMS;

        let mut sms: Vec<StreamingMultiprocessor> = (0..num_sms)
            .map(|i| StreamingMultiprocessor::new(i as u32, Arc::clone(&self.vram)))
            .collect();

        let mut warp_idx = 0usize;
        for block_id in 0..grid_dim {
            let mut block_threads: Vec<SimtHart> = (0..block_dim)
                .map(|tid| SimtHart::new(kern_addr, tid, block_id, block_dim, grid_dim))
                .collect();

            while !block_threads.is_empty() {
                let chunk_end = block_threads.len().min(warp_size);
                let chunk: Vec<SimtHart> = block_threads.drain(..chunk_end).collect();
                sms[warp_idx % num_sms].add_warp(Warp::new(chunk));
                warp_idx += 1;
            }
        }

        let mut total_steps = 0u64;
        loop {
            let mut all_done = true;
            for sm in &mut sms {
                if !sm.is_done() {
                    sm.step();
                    total_steps += 1;
                    all_done = false;
                }
            }
            if all_done {
                break;
            }
        }
        total_steps
    }
}

impl MemTarget for GpgpuCtrl {
    fn target_id(&self) -> u32 {
        0
    }
    fn base(&self) -> u64 {
        0
    }
    fn size(&self) -> u64 {
        256
    }

    fn read(&self, offset: u64, _size: MemAccessSize) -> OmniResult<u64> {
        let val = match offset {
            CMD_STATUS => self.status.load(Ordering::Acquire),
            CMD_OP => self.op.load(Ordering::Acquire),
            CMD_LEN => self.len.load(Ordering::Acquire),
            CMD_SRC_A => self.src_a.load(Ordering::Acquire),
            CMD_SRC_B => self.src_b.load(Ordering::Acquire),
            CMD_DST => self.dst.load(Ordering::Acquire),
            CMD_KERN_ADDR => self.kern_addr.load(Ordering::Acquire),
            CMD_GRID_DIM => self.grid_dim.load(Ordering::Acquire),
            CMD_BLOCK_DIM => self.block_dim.load(Ordering::Acquire),
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
            CMD_OP => {
                self.op.store(v, Ordering::Release);
            }
            CMD_LEN => {
                self.len.store(v, Ordering::Release);
            }
            CMD_SRC_A => {
                self.src_a.store(v, Ordering::Release);
            }
            CMD_SRC_B => {
                self.src_b.store(v, Ordering::Release);
            }
            CMD_DST => {
                self.dst.store(v, Ordering::Release);
            }
            CMD_KERN_ADDR => {
                self.kern_addr.store(v, Ordering::Release);
            }
            CMD_GRID_DIM => {
                self.grid_dim.store(v, Ordering::Release);
            }
            CMD_BLOCK_DIM => {
                self.block_dim.store(v, Ordering::Release);
            }
            _ => {}
        }
        Ok(())
    }
}
