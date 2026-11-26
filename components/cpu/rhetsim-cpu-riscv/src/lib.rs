//! RISC-V RV64IMA CPU component

pub mod cpu;
pub mod csr;
pub mod decode;
pub mod jit;
pub mod mmu;

pub use cpu::{RiscvCpu, RiscvIrqLine};

pub fn register() {}
