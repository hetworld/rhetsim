//! SimClock — 全局仿真时钟

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct SimClock {
    now_ns: Arc<AtomicU64>,
}

impl SimClock {
    pub fn new() -> Self { Self { now_ns: Arc::new(AtomicU64::new(0)) } }
    pub fn now_ns(&self) -> u64 { self.now_ns.load(Ordering::Relaxed) }
    pub fn advance(&self, delta_ns: u64) -> u64 {
        self.now_ns.fetch_add(delta_ns, Ordering::Relaxed) + delta_ns
    }
}

impl Default for SimClock {
    fn default() -> Self { Self::new() }
}
