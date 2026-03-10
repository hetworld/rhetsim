//! machine_map — -machine 名 → 基础 Manifest 模板

use std::collections::HashMap;
use omnisim_manifest::manifest::Manifest;

pub fn builtin_machines() -> HashMap<String, &'static str> {
    let mut m = HashMap::new();
    m.insert("virt".to_string(),         "profiles/server/aarch64-virt.toml");
    m.insert("riscv-soc".to_string(),    "profiles/embedded/riscv-soc.toml");
    m.insert("arm-riscv-gpgpu".to_string(), "profiles/hetero/arm-riscv-gpgpu.toml");
    m
}
