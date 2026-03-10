//! Scheduler — 调度器

use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use omnisim_core::error::OmniResult;
use omnisim_core::unit::UnitId;
use omnisim_manifest::realizer::Platform;
use omnisim_manifest::manifest::UnitDesc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState { Idle, Running, Paused, Stepping, Terminated }

pub struct SchedulerConfig {
    pub exec_budget_ns:      u64,
    pub sync_quantum_ns:     u64,
    pub thread_policy:       ThreadPolicy,
    pub max_vm_exit_retries: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { exec_budget_ns: 1_000_000, sync_quantum_ns: 1_000_000,
               thread_policy: ThreadPolicy::OneThreadPerUnit, max_vm_exit_retries: 3 }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ThreadPolicy {
    #[default] OneThreadPerUnit,
    ThreadPool { size: usize },
}

pub struct Scheduler {
    platform: Arc<RwLock<Platform>>,
    state:    SchedulerState,
    config:   SchedulerConfig,
}

impl Scheduler {
    pub fn new(platform: Arc<RwLock<Platform>>, config: SchedulerConfig) -> Self {
        Self { platform, state: SchedulerState::Idle, config }
    }

    pub fn state(&self) -> SchedulerState { self.state }

    pub fn start(&mut self)  -> OmniResult<()> { self.state = SchedulerState::Running;    Ok(()) }
    pub fn pause(&mut self)  -> OmniResult<()> { self.state = SchedulerState::Paused;     Ok(()) }
    pub fn resume(&mut self) -> OmniResult<()> { self.state = SchedulerState::Running;    Ok(()) }
    pub fn stop(&mut self)   -> OmniResult<()> { self.state = SchedulerState::Terminated; Ok(()) }

    pub fn reset(&mut self) -> OmniResult<()> {
        self.pause()?;
        // TODO: reset all units
        self.resume()
    }

    pub fn hot_add_unit(&mut self, _desc: UnitDesc) -> OmniResult<UnitId> {
        Err(omnisim_core::error::OmniError::NotSupported("hot_add_unit"))
    }

    pub fn hot_remove_unit(&mut self, _id: &UnitId) -> OmniResult<()> {
        Err(omnisim_core::error::OmniError::NotSupported("hot_remove_unit"))
    }
}
