//! TcgFrontend trait — 指令集前端

use omnisim_core::error::OmniResult;
use crate::ir::TcgIrBlock;

pub trait TcgFrontend: Send + Sync {
    fn arch_name(&self) -> &'static str;
    fn translate(&self, pc: u64, mem: &[u8]) -> OmniResult<TcgIrBlock>;
    fn pc_advance(&self, ir: &TcgIrBlock) -> u64;
}

pub trait CpuState: Send + Sync {
    fn read_reg(&self, reg: u32) -> u64;
    fn write_reg(&mut self, reg: u32, val: u64);
    fn pc(&self) -> u64;
    fn set_pc(&mut self, pc: u64);
}
