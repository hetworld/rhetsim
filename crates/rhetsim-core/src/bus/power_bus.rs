//! PowerBus — 电源域管理总线

use std::sync::Arc;
use crate::error::OmniResult;
use crate::unit::UnitId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState { On, Off, Retention }

/// 电源信号接收端
pub trait PowerSink: Send + Sync {
    fn power_on(&self)        -> OmniResult<()>;
    fn power_off(&self)       -> OmniResult<()>;
    fn enter_retention(&self) -> OmniResult<()>;
    fn power_state(&self)     -> PowerState;
}

pub struct PowerConnection {
    pub controller: UnitId,
    pub target:     UnitId,
    pub domain:     String,
}

pub struct PowerBus {
    pub id:          String,
    pub connections: Vec<PowerConnection>,
    sinks:           Vec<(UnitId, Arc<dyn PowerSink>)>,
}

impl PowerBus {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), connections: Vec::new(), sinks: Vec::new() }
    }

    pub fn register_sink(&mut self, unit_id: UnitId, sink: Arc<dyn PowerSink>) {
        self.sinks.push((unit_id, sink));
    }

    pub fn power_on(&self, target: &UnitId) -> OmniResult<()> {
        self.find(target)?.power_on()
    }

    pub fn power_off(&self, target: &UnitId) -> OmniResult<()> {
        self.find(target)?.power_off()
    }

    pub fn enter_retention(&self, target: &UnitId) -> OmniResult<()> {
        self.find(target)?.enter_retention()
    }

    pub fn state(&self, target: &UnitId) -> OmniResult<PowerState> {
        Ok(self.find(target)?.power_state())
    }

    fn find(&self, target: &UnitId) -> OmniResult<&dyn PowerSink> {
        self.sinks.iter()
            .find(|(id, _)| id == target)
            .map(|(_, s)| s.as_ref())
            .ok_or_else(|| crate::error::OmniError::UnitNotFound(target.to_string()))
    }
}
