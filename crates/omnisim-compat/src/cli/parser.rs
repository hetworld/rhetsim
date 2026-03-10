//! QemuCliParser — QEMU 命令行解析

use std::collections::HashMap;
use omnisim_core::error::OmniResult;
use omnisim_manifest::manifest::Manifest;

pub struct DeviceFactory;

pub struct QemuCliParser {
    machine_map: HashMap<String, Manifest>,
}

impl QemuCliParser {
    pub fn new() -> Self { Self { machine_map: HashMap::new() } }

    pub fn parse(&self, args: &[String]) -> OmniResult<Manifest> {
        // TODO: 解析 -machine / -cpu / -m / -smp / -device 等参数
        Err(omnisim_core::error::OmniError::NotSupported("cli parse"))
    }
}

impl Default for QemuCliParser {
    fn default() -> Self { Self::new() }
}
