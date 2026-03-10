//! Command Processor — 解析 Host 下发的命令队列

use omnisim_core::error::OmniResult;

pub struct CommandProcessor;

impl CommandProcessor {
    pub fn new() -> Self { Self }
    pub fn process(&self) -> OmniResult<()> { Ok(()) }
}

impl Default for CommandProcessor {
    fn default() -> Self { Self::new() }
}
