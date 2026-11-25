//! UnitRegistry — 全局单元注册表

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use crate::unit::{Unit, UnitId, UnitCaps};

pub struct UnitRegistry {
    units: HashMap<UnitId, Arc<Mutex<Box<dyn Unit>>>>,
}

impl UnitRegistry {
    pub fn new() -> Self { Self { units: HashMap::new() } }

    pub fn register(&mut self, unit: Box<dyn Unit>) -> UnitId {
        let id = unit.id().clone();
        self.units.insert(id.clone(), Arc::new(Mutex::new(unit)));
        id
    }

    pub fn unregister(&mut self, id: &UnitId) -> bool {
        self.units.remove(id).is_some()
    }

    pub fn get(&self, id: &UnitId) -> Option<Arc<Mutex<Box<dyn Unit>>>> {
        self.units.get(id).cloned()
    }

    pub fn find_by_kind(&self, kind: &str) -> Vec<Arc<Mutex<Box<dyn Unit>>>> {
        self.units.values()
            .filter(|u| u.lock().unwrap().kind() == kind)
            .cloned()
            .collect()
    }

    pub fn find_by_caps(&self, caps: UnitCaps) -> Vec<Arc<Mutex<Box<dyn Unit>>>> {
        self.units.values()
            .filter(|u| u.lock().unwrap().caps().contains(caps))
            .cloned()
            .collect()
    }

    pub fn executable_units(&self) -> Vec<Arc<Mutex<Box<dyn Unit>>>> {
        self.find_by_caps(UnitCaps::EXECUTABLE)
    }

    pub fn all_ids(&self) -> Vec<UnitId> {
        self.units.keys().cloned().collect()
    }
}

impl Default for UnitRegistry {
    fn default() -> Self { Self::new() }
}
