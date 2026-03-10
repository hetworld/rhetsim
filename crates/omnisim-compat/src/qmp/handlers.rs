//! QMP 命令处理器

use omnisim_core::error::OmniResult;
use crate::qmp::types::{QmpRequest, QmpResponse};

pub fn dispatch(req: &QmpRequest) -> OmniResult<QmpResponse> {
    match req.execute.as_str() {
        "qmp_capabilities"  => Ok(QmpResponse { result: serde_json::json!({"capabilities": []}) }),
        "query-version"     => Ok(QmpResponse { result: serde_json::json!({"qemu": {"major":8,"minor":2,"micro":0}}) }),
        "query-status"      => Ok(QmpResponse { result: serde_json::json!({"status":"running","singlestep":false,"running":true}) }),
        "stop"              => Ok(QmpResponse { result: serde_json::json!({}) }),
        "cont"              => Ok(QmpResponse { result: serde_json::json!({}) }),
        "system_reset"      => Ok(QmpResponse { result: serde_json::json!({}) }),
        "system_powerdown"  => Ok(QmpResponse { result: serde_json::json!({}) }),
        _ => Err(omnisim_core::error::OmniError::NotSupported("qmp command")),
    }
}
