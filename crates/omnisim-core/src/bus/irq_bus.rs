//! IrqBus / IrqRouter — 中断总线

use std::sync::Arc;
use std::collections::HashMap;
use crate::error::{OmniError, OmniResult};
use crate::irq::{IrqSink, IrqLine};
use crate::unit::UnitId;

struct IrqConnection {
    from_unit: UnitId,
    from_name: String,
    sink:      Arc<dyn IrqSink>,
    irq_num:   u32,
}

pub struct IrqBus {
    pub id:      String,
    connections: Vec<IrqConnection>,
}

impl IrqBus {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), connections: Vec::new() }
    }

    pub fn connect(
        &mut self,
        from_unit: UnitId,
        from_name: impl Into<String>,
        sink:      Arc<dyn IrqSink>,
        irq_num:   u32,
    ) {
        self.connections.push(IrqConnection {
            from_unit,
            from_name: from_name.into(),
            sink,
            irq_num,
        });
    }
}

/// 全局中断路由器：集中管理所有 IrqBus 的中断连接
pub struct IrqRouter {
    buses: HashMap<String, IrqBus>,
    /// (unit_id, port_name) → IrqLine
    lines: HashMap<(String, String), IrqLine>,
}

impl IrqRouter {
    pub fn new() -> Self {
        Self { buses: HashMap::new(), lines: HashMap::new() }
    }

    pub fn add_bus(&mut self, bus: IrqBus) {
        self.buses.insert(bus.id.clone(), bus);
    }

    /// 按名字获取 IrqLine（realize 阶段调用）
    pub fn get_line(&self, unit_id: &UnitId, port: &str) -> OmniResult<IrqLine> {
        let key = (unit_id.as_str().to_string(), port.to_string());
        self.lines.get(&key)
            .map(|l| IrqLine::new(Arc::clone(&l.sink), l.irq_num))
            .ok_or_else(|| OmniError::IrqNotFound(format!("{}.{}", unit_id, port)))
    }

    pub fn register_line(&mut self, unit_id: &UnitId, port: &str, line: IrqLine) {
        self.lines.insert(
            (unit_id.as_str().to_string(), port.to_string()),
            line,
        );
    }
}

impl Default for IrqRouter {
    fn default() -> Self { Self::new() }
}
