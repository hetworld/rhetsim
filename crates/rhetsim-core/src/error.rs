//! OmniError — 统一错误类型

use thiserror::Error;

/// RhetSim 统一结果类型
pub type OmniResult<T> = Result<T, OmniError>;

/// RhetSim 统一错误枚举
#[derive(Debug, Error)]
pub enum OmniError {
    /// 地址未映射
    #[error("address not mapped: {0:#x}")]
    AddressNotMapped(u64),

    /// 地址映射重叠
    #[error("address overlap: [{0:#x}, {1:#x})")]
    AddressOverlap(u64, u64),

    /// 访问宽度非法
    #[error("invalid access size: {0}")]
    InvalidAccessSize(u8),

    /// 权限不足
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// 违反安全世界隔离
    #[error("security violation: initiator={0}, target={1}")]
    SecurityViolation(String, String),

    /// 找不到指定 Unit
    #[error("unit not found: {0}")]
    UnitNotFound(String),

    /// Unit 已存在
    #[error("unit already exists: {0}")]
    UnitAlreadyExists(String),

    /// Unit realize 失败
    #[error("realize failed: {0}")]
    RealizeFailed(String),

    /// Unit unrealize 失败
    #[error("unrealize failed: {0}")]
    UnrealizeFailed(String),

    /// 找不到指定中断
    #[error("irq not found: {0}")]
    IrqNotFound(String),

    /// 快照操作失败
    #[error("snapshot error: {0}")]
    SnapshotError(String),

    /// 当前实现不支持该操作
    #[error("not supported: {0}")]
    NotSupported(&'static str),

    /// 操作超时
    #[error("timeout: {0}")]
    Timeout(String),

    /// 配置非法
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    /// 内部错误
    #[error("internal error: {0}")]
    Internal(String),

    /// I/O 错误
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
