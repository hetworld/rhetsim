//! TcgEngine<F, B> — TCG 引擎主接口

use crate::frontend::TcgFrontend;
use crate::host_backend::TcgHostBackend;
use crate::optimizer::Optimizer;
use crate::tb_cache::TbCache;

pub struct TcgEngine<F: TcgFrontend, B: TcgHostBackend> {
    /// Reserved for the translation frontend.
    #[allow(dead_code)]
    frontend: F,
    /// Reserved for the host code generator.
    #[allow(dead_code)]
    host_backend: B,
    cache: TbCache,
    /// Reserved for IR optimization.
    #[allow(dead_code)]
    optimizer: Optimizer,
}

impl<F: TcgFrontend, B: TcgHostBackend> TcgEngine<F, B> {
    pub fn new(frontend: F, host_backend: B) -> Self {
        Self {
            frontend,
            host_backend,
            cache: TbCache::new(64 * 1024 * 1024),
            optimizer: Optimizer::new(),
        }
    }

    pub fn flush_tb_cache(&mut self) {
        self.cache.flush_all();
    }
}
