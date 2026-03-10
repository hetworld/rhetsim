//! IrqSink / IrqLine / IrqSource / MsiSender — 中断子系统接口

use std::sync::Arc;
use crate::error::OmniResult;

/// 中断类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrqKind {
    LevelSensitive,
    EdgeTriggered,
}

/// 中断接收端（GIC、PLIC、CLINT 等实现此 trait）
pub trait IrqSink: Send + Sync {
    /// 拉高中断线
    fn raise(&self, irq_num: u32) -> OmniResult<()>;

    /// 拉低中断线
    fn lower(&self, irq_num: u32) -> OmniResult<()>;

    /// 脉冲（raise 后立即 lower，用于边沿触发）
    fn pulse(&self, irq_num: u32) -> OmniResult<()> {
        self.raise(irq_num)?;
        self.lower(irq_num)
    }

    /// 此 Sink 接受的中断线数量
    fn irq_count(&self) -> u32;
}

/// 一条中断线（从外设到 IrqSink 的连接）
pub struct IrqLine {
    pub sink:    Arc<dyn IrqSink>,
    pub irq_num: u32,
}

impl IrqLine {
    pub fn new(sink: Arc<dyn IrqSink>, irq_num: u32) -> Self {
        Self { sink, irq_num }
    }

    pub fn raise(&self) -> OmniResult<()>  { self.sink.raise(self.irq_num) }
    pub fn lower(&self) -> OmniResult<()>  { self.sink.lower(self.irq_num) }
    pub fn pulse(&self) -> OmniResult<()>  { self.sink.pulse(self.irq_num) }
}

/// 中断发出端描述（Unit 声明自己有哪些中断输出）
pub struct IrqSourceDesc {
    pub name: String,
    pub kind: IrqKind,
}

/// 中断发出端 trait
pub trait IrqSource: Send + Sync {
    fn irq_source_descs(&self) -> &[IrqSourceDesc];
}

/// MSI 描述符
#[derive(Debug, Clone, Copy)]
pub struct MsiDesc {
    pub addr:   u64,
    pub data:   u32,
    pub vector: u32,
}

/// MSI 发送接口（PCIe EP 实现，向 RC 发送 MSI/MSI-X）
pub trait MsiSender: Send + Sync {
    fn send_msi(&self, desc: &MsiDesc) -> OmniResult<()>;
}
