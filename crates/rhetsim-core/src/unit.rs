//! Unit trait / UnitId / UnitCaps / RealizeCtx — 核心组合接口

use std::sync::Arc;
use bitflags::bitflags;
use crate::error::OmniResult;
use crate::mem::{MemTarget, MemSpace};
use crate::irq::{IrqSink, IrqSourceDesc};
use crate::dataflow::DataEndpoint;
use crate::backend::execution::ExecutionBackend;
use crate::security::SecurityWorld;
use crate::snapshot::UnitSnapshot;

/// Unit ID：全局唯一，层次化命名
/// 格式："soc0.arm_ss.core0"、"gpgpu0.sm[3].alu"
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitId(String);

impl UnitId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }

    /// 父节点 ID（最后一个 '.' 之前的部分）
    pub fn parent(&self) -> Option<UnitId> {
        self.0.rfind('.').map(|i| UnitId(self.0[..i].to_string()))
    }

    /// 子节点 ID
    pub fn child(&self, name: &str) -> UnitId {
        UnitId(format!("{}.{}", self.0, name))
    }
}

impl std::fmt::Display for UnitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

bitflags! {
    /// Unit 能力标志
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UnitCaps: u32 {
        /// 有独立执行逻辑（实现 ExecutionBackend）
        const EXECUTABLE    = 1 << 0;
        /// 可被地址访问（实现 MemTarget）
        const ADDRESSABLE   = 1 << 1;
        /// 可发出中断
        const IRQ_SOURCE    = 1 << 2;
        /// 可接收中断
        const IRQ_SINK      = 1 << 3;
        /// 复合单元（包含子 Unit）
        const COMPOSITE     = 1 << 4;
        /// 支持调试（实现 DebugBackend）
        const DEBUGGABLE    = 1 << 5;
        /// 支持快照
        const SNAPSHOTTABLE = 1 << 6;
    }
}

/// realize 上下文：注入 Unit 所需的运行时资源
pub struct RealizeCtx<'a> {
    /// 此 Unit 的地址空间视图
    pub mem_space:  Option<MemSpace>,
    /// 全局中断路由器引用
    pub irq_router: &'a mut crate::bus::irq_bus::IrqRouter,
    /// 全局 Unit 注册表引用
    pub registry:   &'a crate::registry::UnitRegistry,
}

impl<'a> RealizeCtx<'a> {
    /// 创建子 Unit 的 RealizeCtx
    pub fn child_ctx(&mut self, _child_id: &UnitId) -> RealizeCtx<'_> {
        RealizeCtx {
            mem_space:  None,
            irq_router: self.irq_router,
            registry:   self.registry,
        }
    }
}

/// 启动步骤
#[derive(Debug, Clone)]
pub enum BootStep {
    /// 拉低复位信号
    AssertReset   { bus: String },
    /// 释放复位信号
    DeassertReset { bus: String, target: Option<UnitId> },
    /// 等待链路就绪
    WaitLinksUp   { bus: String, timeout_ns: u64 },
    /// 等待 Unit 就绪信号
    WaitReady     { unit: UnitId, timeout_ns: u64 },
    /// 上电
    PowerOn       { bus: String, target: UnitId },
    /// 下电
    PowerOff      { bus: String, target: UnitId },
    /// 加载固件
    LoadFirmware  { unit: UnitId, path: String },
}

/// 所有硬件实体的统一抽象
pub trait Unit: Send + Sync {
    fn id(&self)   -> &UnitId;
    fn kind(&self) -> &'static str;
    fn caps(&self) -> UnitCaps;

    /// realize：连接 Bus、获取资源（平台组装时调用一次）
    fn realize(&mut self, _ctx: &mut RealizeCtx) -> OmniResult<()> { Ok(()) }

    /// unrealize：释放资源（热拔出时调用）
    fn unrealize(&mut self) -> OmniResult<()> { Ok(()) }

    /// 复位
    fn reset(&mut self) -> OmniResult<()> { Ok(()) }

    // ── 接口向下转型 ────────────────────────────────────────
    fn as_mem_target(&self) -> Option<&dyn MemTarget>           { None }
    fn as_irq_sink(&self)   -> Option<Arc<dyn IrqSink>>         { None }
    fn as_data_endpoint(&self) -> Option<Arc<dyn DataEndpoint>> { None }
    fn execution_backend(&mut self) -> Option<&mut dyn ExecutionBackend> { None }

    // ── Composite Unit ──────────────────────────────────────
    fn sub_units(&self) -> Option<&[Box<dyn Unit>]>              { None }
    fn sub_units_mut(&mut self) -> Option<&mut Vec<Box<dyn Unit>>> { None }

    // ── 中断源描述 ──────────────────────────────────────────
    fn irq_source_descs(&self) -> &[IrqSourceDesc]               { &[] }

    // ── 快照 ────────────────────────────────────────────────
    fn save_snapshot(&self) -> OmniResult<UnitSnapshot> {
        Err(crate::error::OmniError::NotSupported("snapshot"))
    }
    fn load_snapshot(&mut self, _snap: &UnitSnapshot) -> OmniResult<()> {
        Err(crate::error::OmniError::NotSupported("snapshot"))
    }

    // ── 属性（QMP query 用）─────────────────────────────────
    fn get_property(&self, _key: &str) -> Option<String>         { None }
    fn set_property(&mut self, _key: &str, _val: &str) -> OmniResult<()> {
        Err(crate::error::OmniError::NotSupported("set_property"))
    }

    // ── 安全接口（P0 预留，P2 实现）────────────────────────
    fn security_world(&self) -> SecurityWorld { SecurityWorld::Normal }
    fn switch_world(&mut self, _target: SecurityWorld) -> OmniResult<()> {
        Err(crate::error::OmniError::NotSupported("security world switch"))
    }
    fn current_privilege(&self) -> Option<crate::mem::PrivilegeLevel> { None }

    // ── 电源管理（PowerBus 相关）────────────────────────────
    fn power_sink(&self) -> Option<Arc<dyn crate::bus::power_bus::PowerSink>> { None }

    // ── 复位管理（ResetBus 相关）────────────────────────────
    fn reset_sink(&self) -> Option<Arc<dyn crate::bus::reset_bus::ResetSink>> { None }
}
