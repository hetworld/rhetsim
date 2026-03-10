//! Realizer / ComponentRegistry / Platform — Manifest → 运行时平台

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use omnisim_core::error::OmniResult;
use omnisim_core::unit::{Unit, UnitId};
use omnisim_core::registry::UnitRegistry;
use omnisim_core::bus::{
    mem_bus::MemBus, irq_bus::{IrqBus, IrqRouter},
    data_bus::DataBus, sync_bus::SyncBus,
    pcie_bus::PcieBus, reset_bus::ResetBus,
    power_bus::PowerBus, interconnect_bus::InterconnectBus,
};
use omnisim_core::snapshot::PlatformSnapshot;
use omnisim_core::backend::execution::BackendKind;
use crate::manifest::{Manifest, UnitDesc, BusDesc, BootDesc};
use crate::validator::Validator;
use crate::expander::Expander;

/// 组件工厂函数类型
pub type UnitFactory = Box<dyn Fn(&UnitDesc) -> OmniResult<Box<dyn Unit>> + Send + Sync>;

/// 组件注册表
pub struct ComponentRegistry {
    factories: HashMap<String, UnitFactory>,
    schemas:   HashMap<String, serde_json::Value>,
    backends:  HashMap<String, HashSet<String>>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self { factories: HashMap::new(), schemas: HashMap::new(), backends: HashMap::new() }
    }

    pub fn register(
        &mut self,
        kind:     &'static str,
        factory:  UnitFactory,
        schema:   Option<serde_json::Value>,
        backends: &[BackendKind],
    ) {
        self.factories.insert(kind.to_string(), factory);
        if let Some(s) = schema {
            self.schemas.insert(kind.to_string(), s);
        }
        self.backends.insert(
            kind.to_string(),
            backends.iter().map(|b| format!("{:?}", b)).collect(),
        );
    }

    pub fn get_factory(&self, kind: &str) -> Option<&UnitFactory> {
        self.factories.get(kind)
    }

    pub fn get_schema(&self, kind: &str) -> Option<&serde_json::Value> {
        self.schemas.get(kind)
    }

    pub fn known_kinds(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(|s| s.as_str())
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self { Self::new() }
}

/// 运行时 Bus 集合
pub struct BusSet {
    pub mem_buses:          HashMap<String, MemBus>,
    pub irq_buses:          HashMap<String, IrqBus>,
    pub data_buses:         HashMap<String, DataBus>,
    pub sync_buses:         HashMap<String, SyncBus>,
    pub pcie_buses:         HashMap<String, PcieBus>,
    pub reset_buses:        HashMap<String, ResetBus>,
    pub power_buses:        HashMap<String, PowerBus>,
    pub interconnect_buses: HashMap<String, InterconnectBus>,
}

impl BusSet {
    pub fn new() -> Self {
        Self {
            mem_buses:          HashMap::new(),
            irq_buses:          HashMap::new(),
            data_buses:         HashMap::new(),
            sync_buses:         HashMap::new(),
            pcie_buses:         HashMap::new(),
            reset_buses:        HashMap::new(),
            power_buses:        HashMap::new(),
            interconnect_buses: HashMap::new(),
        }
    }
}

impl Default for BusSet {
    fn default() -> Self { Self::new() }
}

/// 运行时平台 — realize 的产物
pub struct Platform {
    pub name:       String,
    pub unit_reg:   UnitRegistry,
    pub buses:      BusSet,
    pub irq_router: IrqRouter,
    pub boot:       BootDesc,
    pub manifest:   Manifest,
}

impl Platform {
    pub fn hot_add(
        &mut self,
        _unit: UnitDesc,
        _buses: Vec<BusDesc>,
        _realizer: &Realizer,
    ) -> OmniResult<UnitId> {
        Err(omnisim_core::error::OmniError::NotSupported("hot_add"))
    }

    pub fn hot_remove(&mut self, _unit_id: &UnitId) -> OmniResult<()> {
        Err(omnisim_core::error::OmniError::NotSupported("hot_remove"))
    }

    pub fn snapshot(&self) -> OmniResult<PlatformSnapshot> {
        Ok(PlatformSnapshot::new(&self.name))
    }

    pub fn restore(_snap: PlatformSnapshot, _realizer: &Realizer) -> OmniResult<Platform> {
        Err(omnisim_core::error::OmniError::NotSupported("restore"))
    }
}

/// Realizer — 把 Manifest 转换为运行时 Platform
pub struct Realizer {
    registry: Arc<ComponentRegistry>,
}

impl Realizer {
    pub fn new(registry: Arc<ComponentRegistry>) -> Self { Self { registry } }

    pub fn realize(&self, manifest: &Manifest) -> OmniResult<Platform> {
        let validator = Validator::new();
        let report = validator.validate(manifest)?;
        if !report.is_ok() {
            return Err(omnisim_core::error::OmniError::RealizeFailed(
                report.errors.join("; ")
            ));
        }

        let expander = Expander::new();
        let expanded = expander.expand(manifest)?;

        let mut unit_reg   = UnitRegistry::new();
        let mut irq_router = IrqRouter::new();

        for desc in &expanded.units {
            if let Some(factory) = self.registry.get_factory(&desc.kind) {
                let unit = factory(desc)?;
                unit_reg.register(unit);
            }
        }

        Ok(Platform {
            name:       manifest.name.clone(),
            unit_reg,
            buses:      BusSet::new(),
            irq_router,
            boot:       manifest.boot.clone(),
            manifest:   manifest.clone(),
        })
    }
}
