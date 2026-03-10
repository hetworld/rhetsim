//! TcgEngine<F, B> — TCG 引擎主接口

use omnisim_core::error::OmniResult;
use crate::frontend::TcgFrontend;
use crate::host_backend::TcgHostBackend;
use crate::tb_cache::TbCache;
use crate::optimizer::Optimizer;

pub struct TcgEngine<F: TcgFrontend, B: TcgHostBackend> {
    frontend:     F,
    host_backend: B,
    cache:        TbCache,
    optimizer:    Optimizer,
}

impl<F: TcgFrontend, B: TcgHostBackend> TcgEngine<F, B> {
    pub fn new(frontend: F, host_backend: B) -> Self {
        Self { frontend, host_backend, cache: TbCache::new(64 * 1024 * 1024),
               optimizer: Optimizer::new() }
    }

    pub fn flush_tb_cache(&mut self) { self.cache.flush_all(); }
}
