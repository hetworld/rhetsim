//! Mock CPU component — a minimal Unit + ExecutionBackend for testing.

use rhetsim_core::error::{OmniError, OmniResult};
use rhetsim_core::unit::{Unit, UnitId, UnitCaps};
use rhetsim_core::backend::execution::ExecutionBackend;

pub struct MockCpu {
    id: UnitId,
    cycles: u64,
    halted: bool,
}

impl MockCpu {
    pub fn new(id: &str) -> Self {
        Self {
            id: UnitId::new(id),
            cycles: 0,
            halted: false,
        }
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn halt(&mut self) {
        self.halted = true;
    }
}

impl Unit for MockCpu {
    fn id(&self) -> &UnitId { &self.id }
    fn kind(&self) -> &'static str { "mock-cpu" }
    fn caps(&self) -> UnitCaps { UnitCaps::EXECUTABLE }

    fn reset(&mut self) -> OmniResult<()> {
        self.cycles = 0;
        self.halted = false;
        Ok(())
    }

    fn execution_backend(&mut self) -> Option<&mut dyn ExecutionBackend> {
        Some(self)
    }
}

impl ExecutionBackend for MockCpu {
    fn exec(&mut self, budget_ns: u64) -> OmniResult<u64> {
        if self.halted {
            return Ok(0);
        }
        self.cycles += budget_ns;
        Ok(budget_ns)
    }

    fn reset(&mut self) -> OmniResult<()> {
        self.cycles = 0;
        self.halted = false;
        Ok(())
    }

    fn save_state(&self) -> OmniResult<Vec<u8>> {
        Ok(self.cycles.to_le_bytes().to_vec())
    }

    fn load_state(&mut self, data: &[u8]) -> OmniResult<()> {
        if data.len() < 8 {
            return Err(OmniError::SnapshotError(
                "mock-cpu state must be 8 bytes".into(),
            ));
        }
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&data[..8]);
        self.cycles = u64::from_le_bytes(buf);
        Ok(())
    }

    fn backend_name(&self) -> &'static str { "mock" }
}

pub fn register() {}
