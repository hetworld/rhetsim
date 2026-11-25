//! RISC-V RV64I CPU component

pub mod cpu;
pub mod decode;
pub mod mmu;

pub use cpu::RiscvCpu;

pub fn register() {}
