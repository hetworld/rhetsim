//! UnitRegistry — 全局单元注册表

use crate::unit::{Unit, UnitCaps, UnitId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 全局 Unit 注册表
pub struct UnitRegistry {
    units: HashMap<UnitId, Arc<Mutex<Box<dyn Unit>>>>,
}

impl UnitRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        Self {
            units: HashMap::new(),
        }
    }

    /// 注册一个 Unit，返回其 ID
    pub fn register(&mut self, unit: Box<dyn Unit>) -> UnitId {
        let id = unit.id().clone();
        self.units.insert(id.clone(), Arc::new(Mutex::new(unit)));
        id
    }

    /// 注销指定 ID 的 Unit
    pub fn unregister(&mut self, id: &UnitId) -> bool {
        self.units.remove(id).is_some()
    }

    /// 按 ID 获取 Unit
    pub fn get(&self, id: &UnitId) -> Option<Arc<Mutex<Box<dyn Unit>>>> {
        self.units.get(id).cloned()
    }

    /// 按 kind 查找 Unit
    pub fn find_by_kind(&self, kind: &str) -> Vec<Arc<Mutex<Box<dyn Unit>>>> {
        self.units
            .values()
            .filter(|u| u.lock().expect("unit registry mutex poisoned").kind() == kind)
            .cloned()
            .collect()
    }

    /// 按能力标志查找 Unit
    pub fn find_by_caps(&self, caps: UnitCaps) -> Vec<Arc<Mutex<Box<dyn Unit>>>> {
        self.units
            .values()
            .filter(|u| {
                u.lock()
                    .expect("unit registry mutex poisoned")
                    .caps()
                    .contains(caps)
            })
            .cloned()
            .collect()
    }

    /// 返回所有可执行 Unit
    pub fn executable_units(&self) -> Vec<Arc<Mutex<Box<dyn Unit>>>> {
        self.find_by_caps(UnitCaps::EXECUTABLE)
    }

    /// 返回所有 Unit ID
    pub fn all_ids(&self) -> Vec<UnitId> {
        self.units.keys().cloned().collect()
    }
}

impl Default for UnitRegistry {
    fn default() -> Self {
        Self::new()
    }
}
