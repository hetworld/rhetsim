//! RISC-V RV64IMA CPU component

pub mod cpu;
pub mod csr;
pub mod decode;
pub mod jit;
pub mod mmu;
pub mod profile;

pub use cpu::{RiscvCpu, RiscvIrqLine};
pub use profile::{CpuProfile, IsaExtensions, RiscvCpuType};

pub fn register() {}
