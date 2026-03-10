//! QMP 消息类型

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct QmpRequest {
    pub execute: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmpResponse {
    #[serde(rename = "return")]
    pub result: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmpError {
    pub error: QmpErrorBody,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmpErrorBody {
    pub class: String,
    pub desc:  String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmpEvent {
    pub event:     String,
    pub timestamp: QmpTimestamp,
    pub data:      serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmpTimestamp {
    pub seconds:      u64,
    pub microseconds: u64,
}
