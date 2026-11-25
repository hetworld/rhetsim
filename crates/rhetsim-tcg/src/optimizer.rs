//! TCG 优化器 — 常量折叠 / 死代码消除

use crate::ir::TcgIrBlock;

pub struct Optimizer;

impl Optimizer {
    pub fn new() -> Self { Self }
    pub fn optimize(&self, block: TcgIrBlock) -> TcgIrBlock { block }
}

impl Default for Optimizer {
    fn default() -> Self { Self::new() }
}
