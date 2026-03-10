//! ExecutionBackend / DebugBackend — 执行后端接口

use crate::error::OmniResult;

/// 所有"会动"的硬件单元的统一执行接口。
/// CPU、NPU、DPU、DMA 引擎都实现此 trait。
pub trait ExecutionBackend: Send + Sync {
    /// 执行一段时间，返回实际消耗的纳秒数
    fn exec(&mut self, budget_ns: u64) -> OmniResult<u64>;

    /// 复位执行状态
    fn reset(&mut self) -> OmniResult<()>;

    /// 序列化状态（用于快照/迁移）
    fn save_state(&self) -> OmniResult<Vec<u8>>;

    /// 反序列化并恢复状态
    fn load_state(&mut self, data: &[u8]) -> OmniResult<()>;

    /// Backend 名称（用于日志/调试）
    fn backend_name(&self) -> &'static str;

    /// 是否支持暂停
    fn supports_pause(&self) -> bool { false }

    /// 暂停执行
    fn pause(&mut self) -> OmniResult<()> {
        Err(crate::error::OmniError::NotSupported("pause"))
    }

    /// 恢复执行
    fn resume(&mut self) -> OmniResult<()> {
        Err(crate::error::OmniError::NotSupported("resume"))
    }

    /// 上报事件（VM exit、中断等）
    fn poll_event(&mut self) -> Option<BackendEvent> { None }
}

/// 调试接口扩展（CPU 类 Unit 实现）
pub trait DebugBackend: ExecutionBackend {
    /// 读取寄存器
    fn read_reg(&self, reg_id: u64) -> OmniResult<u64>;

    /// 写入寄存器
    fn write_reg(&mut self, reg_id: u64, val: u64) -> OmniResult<()>;

    /// 当前 PC
    fn pc(&self) -> u64;

    /// 插入断点
    fn insert_breakpoint(&mut self, addr: u64) -> OmniResult<()>;

    /// 移除断点
    fn remove_breakpoint(&mut self, addr: u64) -> OmniResult<()>;

    /// 单步执行
    fn single_step(&mut self) -> OmniResult<StepResult>;
}

/// 单步执行结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// 正常步进
    Ok,
    /// 命中断点
    Breakpoint(u64),
    /// 发生异常
    Exception(u32),
}

/// Backend 类型标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendKind {
    /// 功能仿真（默认）
    #[default]
    Functional,
    /// 时序仿真
    Timing,
    /// RTL 协同
    Rtl,
    /// KVM 硬件虚拟化
    Kvm,
    /// vhost 卸载
    Vhost,
    /// Mock（测试用）
    Mock,
}

/// Backend 上报事件
#[derive(Debug)]
pub enum BackendEvent {
    /// VM exit（KVM）
    VmExit { reason: u32, addr: u64 },
    /// MMIO 访问请求
    MmioAccess { addr: u64, size: u8, is_write: bool, data: u64 },
    /// 中断注入完成
    IrqDelivered { irq: u32 },
    /// 执行停止
    Halted,
}
