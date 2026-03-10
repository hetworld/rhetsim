//! Streaming Multiprocessor — SimtHart 执行阵列

use omnisim_core::error::OmniResult;

pub struct StreamingMultiprocessor {
    pub id:       u32,
    pub num_warps: u32,
}

impl StreamingMultiprocessor {
    pub fn new(id: u32, num_warps: u32) -> Self { Self { id, num_warps } }

    pub fn exec(&mut self, _budget_ns: u64) -> OmniResult<u64> {
        Ok(_budget_ns)
    }
}
