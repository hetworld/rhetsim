//! UnitSnapshot / PlatformSnapshot — 快照接口

use crate::unit::UnitId;

/// 单个 Unit 的状态快照
#[derive(Debug, Clone)]
pub struct UnitSnapshot {
    /// 所属 Unit ID
    pub unit_id: UnitId,
    /// 快照格式版本
    pub version: u32,
    /// Unit 私有快照数据
    pub data: Vec<u8>,
}

/// 整个平台的状态快照
#[derive(Debug, Clone)]
pub struct PlatformSnapshot {
    /// 平台名称
    pub platform_name: String,
    /// 快照时间戳（纳秒）
    pub timestamp_ns: u64,
    /// 各 Unit 的快照
    pub units: Vec<UnitSnapshot>,
}

impl PlatformSnapshot {
    /// 创建空的平台快照
    pub fn new(platform_name: impl Into<String>) -> Self {
        Self {
            platform_name: platform_name.into(),
            timestamp_ns: 0,
            units: Vec::new(),
        }
    }

    /// 添加一个 Unit 快照
    pub fn add_unit(&mut self, snap: UnitSnapshot) {
        self.units.push(snap);
    }

    /// 按 Unit ID 查找快照
    pub fn find_unit(&self, id: &UnitId) -> Option<&UnitSnapshot> {
        self.units.iter().find(|s| &s.unit_id == id)
    }
}
