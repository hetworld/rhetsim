//! VRAM Controller — GDDR6 显存控制器

use rhetsim_core::error::OmniResult;

pub struct VramController {
    pub capacity_bytes: usize,
}

impl VramController {
    pub fn new(capacity_bytes: usize) -> Self { Self { capacity_bytes } }
}
