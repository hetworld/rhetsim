//! Loader — JSON / TOML 序列化与反序列化

use std::path::Path;
use rhetsim_core::error::{OmniError, OmniResult};
use crate::manifest::Manifest;

pub fn load_from_file(path: &Path) -> OmniResult<Manifest> {
    let content = std::fs::read_to_string(path)?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => from_toml(&content),
        Some("json") => from_json(&content),
        Some(ext) => Err(OmniError::InvalidConfig(
            format!("unsupported manifest format: .{ext}"),
        )),
        None => from_toml(&content),
    }
}

pub fn load_from_str(s: &str) -> OmniResult<Manifest> {
    from_toml(s).or_else(|_| from_json(s))
}

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
