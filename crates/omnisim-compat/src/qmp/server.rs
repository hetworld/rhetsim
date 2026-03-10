//! QmpServer — QMP 协议服务端

use omnisim_core::error::OmniResult;

pub struct QmpServer {
    bind_addr: String,
}

impl QmpServer {
    pub fn new(bind_addr: impl Into<String>) -> Self {
        Self { bind_addr: bind_addr.into() }
    }

    pub async fn serve(&self) -> OmniResult<()> {
        tracing::info!("QMP server listening on {}", self.bind_addr);
        Ok(())
    }
}
