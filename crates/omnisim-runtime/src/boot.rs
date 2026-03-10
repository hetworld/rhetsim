//! BootSequencer — 启动顺序执行器

use omnisim_core::error::OmniResult;

pub struct BootSequencer;

impl BootSequencer {
    pub fn new() -> Self { Self }
    pub fn run(&self) -> OmniResult<()> {
        tracing::info!("BootSequencer: starting boot sequence");
        Ok(())
    }
}

impl Default for BootSequencer {
    fn default() -> Self { Self::new() }
}
