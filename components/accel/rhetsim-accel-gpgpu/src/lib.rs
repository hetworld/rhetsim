//! GPGPU — functional accelerator with control registers, VRAM, and command processing.

pub mod cmd_proc;
pub mod sm;
pub mod vram_ctrl;

use crate::cmd_proc::GpgpuCtrl;
use crate::vram_ctrl::GpgpuVram;
use rhetsim_core::backend::execution::ExecutionBackend;
use rhetsim_core::error::OmniResult;
use rhetsim_core::mem::MemTarget;
use rhetsim_core::unit::{Unit, UnitCaps, UnitId};
use std::sync::Arc;

pub struct Gpgpu {
    id: UnitId,
    ctrl: Arc<GpgpuCtrl>,
    vram: Arc<GpgpuVram>,
}

impl Gpgpu {
    pub fn new(id: &str, vram_size: usize) -> Self {
        let vram = Arc::new(GpgpuVram::new(vram_size));
        let ctrl = Arc::new(GpgpuCtrl::new(Arc::clone(&vram)));
        Self {
            id: UnitId::new(id),
            ctrl,
            vram,
        }
    }

    pub fn ctrl(&self) -> &GpgpuCtrl {
        &self.ctrl
    }
    pub fn vram(&self) -> &GpgpuVram {
        self.vram.as_ref()
    }

    pub fn ctrl_arc(&self) -> Arc<GpgpuCtrl> {
        Arc::clone(&self.ctrl)
    }
    pub fn vram_arc(&self) -> Arc<GpgpuVram> {
        Arc::clone(&self.vram)
    }
}

impl Unit for Gpgpu {
    fn id(&self) -> &UnitId {
        &self.id
    }
    fn kind(&self) -> &'static str {
        "gpgpu"
    }
    fn caps(&self) -> UnitCaps {
        UnitCaps::EXECUTABLE | UnitCaps::ADDRESSABLE
    }

    fn execution_backend(&mut self) -> Option<&mut dyn ExecutionBackend> {
        Some(self)
    }

    fn reset(&mut self) -> OmniResult<()> {
        Ok(())
    }
}

impl ExecutionBackend for Gpgpu {
    fn exec(&mut self, _budget_ns: u64) -> OmniResult<u64> {
        self.ctrl.process_command()
    }

    fn reset(&mut self) -> OmniResult<()> {
        Unit::reset(self)
    }

    fn save_state(&self) -> OmniResult<Vec<u8>> {
        Ok(Vec::new())
    }

    fn load_state(&mut self, _data: &[u8]) -> OmniResult<()> {
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "gpgpu-functional"
    }
}

pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;
    use rhetsim_core::mem::MemAccessSize;

    #[test]
    fn test_gpgpu_vram_read_write() {
        let gpu = Gpgpu::new("gpu0", 64 * 1024);
        gpu.vram().write(0, MemAccessSize::U32, 0xDEADBEEF).unwrap();
        assert_eq!(gpu.vram().read(0, MemAccessSize::U32).unwrap(), 0xDEADBEEF);

        gpu.vram()
            .write(100, MemAccessSize::U64, 0x0123456789ABCDEF)
            .unwrap();
        assert_eq!(
            gpu.vram().read(100, MemAccessSize::U64).unwrap(),
            0x0123456789ABCDEF
        );
    }

    #[test]
    fn test_gpgpu_idle() {
        let mut gpu = Gpgpu::new("gpu0", 64 * 1024);
        let consumed = gpu.exec(1000).unwrap();
        assert_eq!(consumed, 0);
    }

    #[test]
    fn test_gpgpu_vector_add() {
        let mut gpu = Gpgpu::new("gpu0", 64 * 1024);

        let src_a_off: u64 = 0x0000;
        let src_b_off: u64 = 0x1000;
        let dst_off: u64 = 0x2000;
        let len: u32 = 4;

        let vec_a: [u32; 4] = [1, 2, 3, 4];
        let vec_b: [u32; 4] = [10, 20, 30, 40];

        for (i, &v) in vec_a.iter().enumerate() {
            gpu.vram().write_u32(src_a_off + (i as u64) * 4, v).unwrap();
        }
        for (i, &v) in vec_b.iter().enumerate() {
            gpu.vram().write_u32(src_b_off + (i as u64) * 4, v).unwrap();
        }

        // Set up command registers
        gpu.ctrl()
            .write(0x08, MemAccessSize::U32, cmd_proc::OP_VECTOR_ADD as u64)
            .unwrap();
        gpu.ctrl()
            .write(0x0C, MemAccessSize::U32, len as u64)
            .unwrap();
        gpu.ctrl()
            .write(0x10, MemAccessSize::U32, src_a_off)
            .unwrap();
        gpu.ctrl()
            .write(0x14, MemAccessSize::U32, src_b_off)
            .unwrap();
        gpu.ctrl().write(0x18, MemAccessSize::U32, dst_off).unwrap();
        // Ring doorbell
        gpu.ctrl().write(0x00, MemAccessSize::U32, 1).unwrap();

        let consumed = gpu.exec(1000).unwrap();
        assert_eq!(consumed, 4);

        // Verify status
        let status = gpu.ctrl().read(0x04, MemAccessSize::U32).unwrap();
        assert_eq!(status, cmd_proc::STATUS_DONE as u64);

        // Verify result
        let expected: [u32; 4] = [11, 22, 33, 44];
        for (i, &exp) in expected.iter().enumerate() {
            let val = gpu.vram().read_u32(dst_off + (i as u64) * 4).unwrap();
            assert_eq!(val, exp, "mismatch at index {i}");
        }
    }

    #[test]
    fn test_gpgpu_kernel_dispatch() {
        let mut gpu = Gpgpu::new("gpu0", 0x10000);

        // Load kernel: result[global_tid] = vec_a[global_tid] + vec_b[global_tid]
        // global_tid = block_id * block_dim + thread_id
        // a0=tid, a1=block_id, a2=block_dim
        // vec_a at 0x4000, vec_b at 0x5000, result at 0x6000
        let kernel: Vec<u32> = vec![
            0x02C580B3, // MUL  x1, x11, x12     ; x1 = block_id * block_dim
            0x00A080B3, // ADD  x1, x1, x10      ; x1 = global_tid
            0x00209113, // SLLI x2, x1, 2         ; x2 = global_tid * 4
            0x000042B7, // LUI  x5, 0x4
            0x002282B3, // ADD  x5, x5, x2
            0x0002A183, // LW   x3, 0(x5)         ; vec_a[gtid]
            0x00005337, // LUI  x6, 0x5
            0x00230333, // ADD  x6, x6, x2
            0x00032203, // LW   x4, 0(x6)         ; vec_b[gtid]
            0x004183B3, // ADD  x7, x3, x4
            0x00006437, // LUI  x8, 0x6
            0x00240433, // ADD  x8, x8, x2
            0x00742023, // SW   x7, 0(x8)         ; result[gtid]
            0x00000073, // ECALL
        ];
        for (i, &insn) in kernel.iter().enumerate() {
            gpu.vram().write_u32((i * 4) as u64, insn).unwrap();
        }

        // Write input vectors
        let vec_a: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        let vec_b: [u32; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
        for (i, &v) in vec_a.iter().enumerate() {
            gpu.vram().write_u32(0x4000 + (i as u64) * 4, v).unwrap();
        }
        for (i, &v) in vec_b.iter().enumerate() {
            gpu.vram().write_u32(0x5000 + (i as u64) * 4, v).unwrap();
        }

        // Dispatch kernel: 2 blocks × 4 threads
        gpu.ctrl()
            .write(0x08, MemAccessSize::U32, cmd_proc::OP_KERNEL as u64)
            .unwrap();
        gpu.ctrl().write(0x1C, MemAccessSize::U32, 0).unwrap(); // kernel at VRAM 0
        gpu.ctrl().write(0x20, MemAccessSize::U32, 2).unwrap(); // grid_dim = 2
        gpu.ctrl().write(0x24, MemAccessSize::U32, 4).unwrap(); // block_dim = 4
        gpu.ctrl().write(0x00, MemAccessSize::U32, 1).unwrap(); // doorbell

        let consumed = gpu.exec(1000).unwrap();
        assert!(consumed > 0);

        let status = gpu.ctrl().read(0x04, MemAccessSize::U32).unwrap();
        assert_eq!(status, cmd_proc::STATUS_DONE as u64);

        let expected: [u32; 8] = [11, 22, 33, 44, 55, 66, 77, 88];
        for (i, &exp) in expected.iter().enumerate() {
            let val = gpu.vram().read_u32(0x6000 + (i as u64) * 4).unwrap();
            assert_eq!(val, exp, "mismatch at index {i}");
        }
    }

    #[test]
    fn test_gpgpu_vector_mul() {
        let mut gpu = Gpgpu::new("gpu0", 64 * 1024);

        let src_a_off: u64 = 0x0000;
        let src_b_off: u64 = 0x1000;
        let dst_off: u64 = 0x2000;

        let vec_a: [u32; 3] = [2, 3, 5];
        let vec_b: [u32; 3] = [7, 11, 13];

        for (i, &v) in vec_a.iter().enumerate() {
            gpu.vram().write_u32(src_a_off + (i as u64) * 4, v).unwrap();
        }
        for (i, &v) in vec_b.iter().enumerate() {
            gpu.vram().write_u32(src_b_off + (i as u64) * 4, v).unwrap();
        }

        gpu.ctrl()
            .write(0x08, MemAccessSize::U32, cmd_proc::OP_VECTOR_MUL as u64)
            .unwrap();
        gpu.ctrl().write(0x0C, MemAccessSize::U32, 3).unwrap();
        gpu.ctrl()
            .write(0x10, MemAccessSize::U32, src_a_off)
            .unwrap();
        gpu.ctrl()
            .write(0x14, MemAccessSize::U32, src_b_off)
            .unwrap();
        gpu.ctrl().write(0x18, MemAccessSize::U32, dst_off).unwrap();
        gpu.ctrl().write(0x00, MemAccessSize::U32, 1).unwrap();

        let consumed = gpu.exec(1000).unwrap();
        assert_eq!(consumed, 3);

        let expected: [u32; 3] = [14, 33, 65];
        for (i, &exp) in expected.iter().enumerate() {
            let val = gpu.vram().read_u32(dst_off + (i as u64) * 4).unwrap();
            assert_eq!(val, exp, "mismatch at index {i}");
        }
    }
}
