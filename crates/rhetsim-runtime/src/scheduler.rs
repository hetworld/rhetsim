//! Scheduler — round-robin execution driver for all executable Units

use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::Unit;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    Idle,
    Running,
    Paused,
    Terminated,
}

#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    pub total_ns_elapsed: u64,
    pub quanta_executed: u64,
    pub units_count: usize,
}

pub struct Scheduler {
    units: Vec<Arc<Mutex<Box<dyn Unit>>>>,
    state: SchedulerState,
    quantum_ns: u64,
}

impl Scheduler {
    pub fn new(quantum_ns: u64) -> Self {
        Self {
            units: Vec::new(),
            state: SchedulerState::Idle,
            quantum_ns,
        }
    }

    pub fn add_unit(&mut self, unit: Arc<Mutex<Box<dyn Unit>>>) {
        self.units.push(unit);
    }

    pub fn state(&self) -> SchedulerState {
        self.state
    }

    pub fn units_count(&self) -> usize {
        self.units.len()
    }

    /// Execute one quantum: drive every executable unit's backend for up to `quantum_ns`.
    pub fn run_quantum(&mut self, quantum_ns: u64) -> OmniResult<SchedulerStats> {
        self.state = SchedulerState::Running;
        let mut stats = SchedulerStats {
            units_count: self.units.len(),
            ..Default::default()
        };

        for unit_arc in &self.units {
            let mut unit = unit_arc
                .lock()
                .map_err(|e| OmniError::Internal(format!("unit lock poisoned: {e}")))?;
            if let Some(backend) = unit.execution_backend() {
                let consumed = backend.exec(quantum_ns)?;
                stats.total_ns_elapsed += consumed;
            }
        }

        stats.quanta_executed = 1;
        Ok(stats)
    }

    /// Run the simulation for `total_ns`, advancing in `quantum_ns` steps.
    pub fn run(&mut self, total_ns: u64, quantum_ns: u64) -> OmniResult<SchedulerStats> {
        let mut aggregate = SchedulerStats {
            units_count: self.units.len(),
            ..Default::default()
        };
        let mut remaining = total_ns;

        while remaining > 0 {
            if self.state == SchedulerState::Terminated {
                break;
            }
            let budget = remaining.min(quantum_ns);
            let step = self.run_quantum(budget)?;
            aggregate.total_ns_elapsed += step.total_ns_elapsed;
            aggregate.quanta_executed += 1;
            remaining = remaining.saturating_sub(budget);
        }

        self.state = SchedulerState::Idle;
        Ok(aggregate)
    }

    /// Run using the default quantum configured at construction.
    pub fn run_default(&mut self, total_ns: u64) -> OmniResult<SchedulerStats> {
        self.run(total_ns, self.quantum_ns)
    }

    pub fn pause(&mut self) {
        self.state = SchedulerState::Paused;
    }

    pub fn stop(&mut self) {
        self.state = SchedulerState::Terminated;
    }
}
