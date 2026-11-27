//! TcgHostBackend trait — 宿主代码生成后端

use crate::ir::TcgIrBlock;
use rhetsim_core::error::OmniResult;

pub trait TcgHostBackend: Send + Sync {
    fn arch_name(&self) -> &'static str;
    fn compile(&self, block: &TcgIrBlock) -> OmniResult<Vec<u8>>;
}
