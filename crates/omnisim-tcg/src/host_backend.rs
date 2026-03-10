//! TcgHostBackend trait — 宿主代码生成后端

use omnisim_core::error::OmniResult;
use crate::ir::TcgIrBlock;

pub trait TcgHostBackend: Send + Sync {
    fn arch_name(&self) -> &'static str;
    fn compile(&self, block: &TcgIrBlock) -> OmniResult<Vec<u8>>;
}
