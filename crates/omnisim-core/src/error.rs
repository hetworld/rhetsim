//! OmniError — 统一错误类型

use thiserror::Error;

/// OmniSim 统一结果类型
pub type OmniResult<T> = Result<T, OmniError>;

/// OmniSim 统一错误枚举
#[derive(Debug, Error)]
pub enum OmniError {
    #[error("address not mapped: {0:#x}")]
    AddressNotMapped(u64),

    #[error("address overlap: [{0:#x}, {1:#x})")]
    AddressOverlap(u64, u64),

    #[error("invalid access size: {0}")]
    InvalidAccessSize(u8),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("security violation: initiator={0}, target={1}")]
    SecurityViolation(String, String),

    #[error("unit not found: {0}")]
    UnitNotFound(String),

    #[error("unit already exists: {0}")]
    UnitAlreadyExists(String),

    #[error("realize failed: {0}")]
    RealizeFailed(String),

    #[error("unrealize failed: {0}")]
    UnrealizeFailed(String),

    #[error("irq not found: {0}")]
    IrqNotFound(String),

    #[error("snapshot error: {0}")]
    SnapshotError(String),

    #[error("not supported: {0}")]
    NotSupported(&'static str),

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
