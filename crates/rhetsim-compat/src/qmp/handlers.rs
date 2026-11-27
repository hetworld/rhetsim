//! QMP 命令处理器

use crate::qmp::types::{QmpRequest, QmpResponse};
use rhetsim_core::error::OmniResult;

pub struct QmpHandler {
    running: bool,
    quit_requested: bool,
}

impl QmpHandler {
    pub fn new() -> Self {
        Self {
            running: true,
            quit_requested: false,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
    pub fn is_quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn dispatch(&mut self, req: &QmpRequest) -> OmniResult<QmpResponse> {
        match req.execute.as_str() {
            "qmp_capabilities" => Ok(QmpResponse {
                result: serde_json::json!({}),
            }),
            "query-version" => Ok(QmpResponse {
                result: serde_json::json!({
                    "qemu": {"major": 8, "minor": 2, "micro": 0},
                    "package": "rhetsim"
                }),
            }),
            "query-status" => {
                let status = if self.running { "running" } else { "paused" };
                Ok(QmpResponse {
                    result: serde_json::json!({
                        "status": status,
                        "singlestep": false,
                        "running": self.running
                    }),
                })
            }
            "stop" => {
                self.running = false;
                Ok(QmpResponse {
                    result: serde_json::json!({}),
                })
            }
            "cont" => {
                self.running = true;
                Ok(QmpResponse {
                    result: serde_json::json!({}),
                })
            }
            "quit" => {
                self.quit_requested = true;
                self.running = false;
                Ok(QmpResponse {
                    result: serde_json::json!({}),
                })
            }
            "system_reset" => Ok(QmpResponse {
                result: serde_json::json!({}),
            }),
            "system_powerdown" => {
                self.running = false;
                Ok(QmpResponse {
                    result: serde_json::json!({}),
                })
            }
            "query-commands" => {
                let cmds = [
                    "qmp_capabilities",
                    "query-version",
                    "query-status",
                    "stop",
                    "cont",
                    "quit",
                    "system_reset",
                    "system_powerdown",
                ];
                let list: Vec<_> = cmds
                    .iter()
                    .map(|c| serde_json::json!({"name": c}))
                    .collect();
                Ok(QmpResponse {
                    result: serde_json::json!(list),
                })
            }
            _ => Err(rhetsim_core::error::OmniError::NotSupported("qmp command")),
        }
    }
}

impl Default for QmpHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cmd: &str) -> QmpRequest {
        QmpRequest {
            execute: cmd.to_string(),
            arguments: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_initial_state_running() {
        let mut h = QmpHandler::new();
        let resp = h.dispatch(&req("query-status")).unwrap();
        assert_eq!(resp.result["running"], true);
        assert_eq!(resp.result["status"], "running");
    }

    #[test]
    fn test_stop_then_cont() {
        let mut h = QmpHandler::new();

        h.dispatch(&req("stop")).unwrap();
        let resp = h.dispatch(&req("query-status")).unwrap();
        assert_eq!(resp.result["running"], false);
        assert_eq!(resp.result["status"], "paused");

        h.dispatch(&req("cont")).unwrap();
        let resp = h.dispatch(&req("query-status")).unwrap();
        assert_eq!(resp.result["running"], true);
        assert_eq!(resp.result["status"], "running");
    }

    #[test]
    fn test_quit() {
        let mut h = QmpHandler::new();
        assert!(!h.is_quit_requested());

        h.dispatch(&req("quit")).unwrap();
        assert!(h.is_quit_requested());
        assert!(!h.is_running());
    }

    #[test]
    fn test_qmp_capabilities() {
        let mut h = QmpHandler::new();
        let resp = h.dispatch(&req("qmp_capabilities")).unwrap();
        assert_eq!(resp.result, serde_json::json!({}));
    }

    #[test]
    fn test_query_version() {
        let mut h = QmpHandler::new();
        let resp = h.dispatch(&req("query-version")).unwrap();
        assert_eq!(resp.result["qemu"]["major"], 8);
        assert_eq!(resp.result["package"], "rhetsim");
    }

    #[test]
    fn test_query_commands() {
        let mut h = QmpHandler::new();
        let resp = h.dispatch(&req("query-commands")).unwrap();
        let list = resp.result.as_array().unwrap();
        assert!(list.len() >= 8);
        let names: Vec<&str> = list.iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"stop"));
        assert!(names.contains(&"cont"));
        assert!(names.contains(&"quit"));
    }

    #[test]
    fn test_unknown_command() {
        let mut h = QmpHandler::new();
        assert!(h.dispatch(&req("nonexistent")).is_err());
    }

    #[test]
    fn test_system_powerdown() {
        let mut h = QmpHandler::new();
        h.dispatch(&req("system_powerdown")).unwrap();
        assert!(!h.is_running());
    }
}
