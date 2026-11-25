//! Streaming Multiprocessor — placeholder for future SIMT execution.

pub struct StreamingMultiprocessor {
    pub id: u32,
    pub num_warps: u32,
}

impl StreamingMultiprocessor {
    pub fn new(id: u32, num_warps: u32) -> Self { Self { id, num_warps } }
}
