//! Loader — JSON / TOML 序列化与反序列化

use omnisim_core::error::{OmniError, OmniResult};
use crate::manifest::Manifest;

pub fn from_json(s: &str) -> OmniResult<Manifest> {
    serde_json::from_str(s).map_err(|e| OmniError::InvalidConfig(e.to_string()))
}

pub fn to_json(m: &Manifest) -> OmniResult<String> {
    serde_json::to_string_pretty(m).map_err(|e| OmniError::InvalidConfig(e.to_string()))
}

pub fn from_toml(s: &str) -> OmniResult<Manifest> {
    toml::from_str(s).map_err(|e| OmniError::InvalidConfig(e.to_string()))
}

pub fn to_toml(m: &Manifest) -> OmniResult<String> {
    toml::to_string(m).map_err(|e| OmniError::InvalidConfig(e.to_string()))
}
