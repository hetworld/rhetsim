//! TbCache — Translation Block 缓存

use std::collections::HashMap;
use std::sync::Arc;

const FAST_CACHE_SIZE: usize = 1 << 14;

#[derive(Clone)]
pub struct CompiledTb {
    pub guest_pc: u64,
    pub code:     Vec<u8>,
    pub size:     usize,
}

struct CachedTb { pc: u64, tb: Arc<CompiledTb> }

pub struct TbCacheStats {
    pub hits:   u64,
    pub misses: u64,
    pub evictions: u64,
}

pub struct TbCache {
    fast:       Vec<Option<CachedTb>>,
    slow:       HashMap<u64, Arc<CompiledTb>>,
    max_bytes:  usize,
    used_bytes: usize,
    stats:      TbCacheStats,
}

impl TbCache {
    pub fn new(max_bytes: usize) -> Self {
        let mut fast = Vec::with_capacity(FAST_CACHE_SIZE);
        fast.resize_with(FAST_CACHE_SIZE, || None);
        Self { fast, slow: HashMap::new(), max_bytes, used_bytes: 0,
               stats: TbCacheStats { hits: 0, misses: 0, evictions: 0 } }
    }

    pub fn lookup(&mut self, pc: u64) -> Option<Arc<CompiledTb>> {
        let slot = (pc as usize) & (FAST_CACHE_SIZE - 1);
        if let Some(c) = &self.fast[slot] {
            if c.pc == pc { self.stats.hits += 1; return Some(Arc::clone(&c.tb)); }
        }
        let result = self.slow.get(&pc).cloned();
        if result.is_some() { self.stats.hits += 1; } else { self.stats.misses += 1; }
        result
    }

    pub fn insert(&mut self, pc: u64, tb: CompiledTb) -> Arc<CompiledTb> {
        let arc = Arc::new(tb);
        let slot = (pc as usize) & (FAST_CACHE_SIZE - 1);
        self.fast[slot] = Some(CachedTb { pc, tb: Arc::clone(&arc) });
        self.slow.insert(pc, Arc::clone(&arc));
        arc
    }

    pub fn invalidate_range(&mut self, start: u64, end: u64) {
        self.slow.retain(|&pc, _| pc < start || pc >= end);
    }

    pub fn flush_all(&mut self) {
        self.slow.clear();
        for slot in &mut self.fast { *slot = None; }
    }

    pub fn stats(&self) -> &TbCacheStats { &self.stats }
}
