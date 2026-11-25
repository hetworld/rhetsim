//! UnitSnapshot / PlatformSnapshot — 快照接口

use crate::unit::UnitId;

/// 单个 Unit 的状态快照
#[derive(Debug, Clone)]
pub struct UnitSnapshot {
    pub unit_id: UnitId,
    pub version: u32,
    pub data:    Vec<u8>,
}

/// 整个平台的状态快照
#[derive(Debug, Clone)]
pub struct PlatformSnapshot {
    pub platform_name: String,
    pub timestamp_ns:  u64,
    pub units:         Vec<UnitSnapshot>,
}

impl PlatformSnapshot {
    pub fn new(platform_name: impl Into<String>) -> Self {
        Self {
            platform_name: platform_name.into(),
            timestamp_ns:  0,
            units:         Vec::new(),
        }
    }

    pub fn add_unit(&mut self, snap: UnitSnapshot) {
        self.units.push(snap);
    }

    pub fn find_unit(&self, id: &UnitId) -> Option<&UnitSnapshot> {
        self.units.iter().find(|s| &s.unit_id == id)
    }
}
