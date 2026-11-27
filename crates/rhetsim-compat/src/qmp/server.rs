//! QmpServer — QMP 协议服务端

use crate::qmp::handlers::QmpHandler;
use crate::qmp::types::{QmpError, QmpErrorBody, QmpRequest};
use rhetsim_core::error::OmniResult;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

pub struct QmpServer {
    bind_addr: String,
    handler: Arc<Mutex<QmpHandler>>,
}

impl QmpServer {
    pub fn new(bind_addr: impl Into<String>, handler: Arc<Mutex<QmpHandler>>) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            handler,
        }
    }

    pub async fn serve(&self) -> OmniResult<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("QMP server listening on {}", self.bind_addr);

        loop {
            let (stream, addr) = listener.accept().await?;
            tracing::info!("QMP client connected from {}", addr);
            let handler = Arc::clone(&self.handler);
            tokio::spawn(async move {
                if let Err(e) = Self::handle_client(stream, handler).await {
                    tracing::error!("QMP client error: {}", e);
                }
            });
        }
    }

    async fn handle_client(
        stream: tokio::net::TcpStream,
        handler: Arc<Mutex<QmpHandler>>,
    ) -> OmniResult<()> {
        let (reader, mut writer) = stream.into_split();

        let greeting = serde_json::json!({
            "QMP": {
                "version": {
                    "qemu": {"major": 8, "minor": 2, "micro": 0},
                    "package": "rhetsim"
                },
                "capabilities": []
            }
        });
        let greeting_str = serde_json::to_string(&greeting)
            .map_err(|e| rhetsim_core::error::OmniError::Internal(e.to_string()))?;
        writer.write_all(greeting_str.as_bytes()).await?;
        writer.write_all(b"\n").await?;

        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            let req: QmpRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let err = QmpError {
                        error: QmpErrorBody {
                            class: "GenericError".into(),
                            desc: e.to_string(),
                        },
                    };
                    let err_str = serde_json::to_string(&err)
                        .map_err(|e| rhetsim_core::error::OmniError::Internal(e.to_string()))?;
                    writer.write_all(err_str.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    continue;
                }
            };

            let should_quit = req.execute == "quit";

            let resp = handler
                .lock()
                .map_err(|e| rhetsim_core::error::OmniError::Internal(e.to_string()))?
                .dispatch(&req);

            match resp {
                Ok(r) => {
                    let s = serde_json::to_string(&r)
                        .map_err(|e| rhetsim_core::error::OmniError::Internal(e.to_string()))?;
                    writer.write_all(s.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                }
                Err(e) => {
                    let err = QmpError {
                        error: QmpErrorBody {
                            class: "CommandNotFound".into(),
                            desc: e.to_string(),
                        },
                    };
                    let s = serde_json::to_string(&err)
                        .map_err(|e| rhetsim_core::error::OmniError::Internal(e.to_string()))?;
                    writer.write_all(s.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                }
            }

            if should_quit {
                break;
            }
        }

        tracing::info!("QMP client disconnected");
        Ok(())
    }
}
