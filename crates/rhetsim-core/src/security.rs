//! 安全执行接口（P0 预留，P2 实施）
/// 内存访问的安全属性
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemSecurity {
    /// 非安全世界
    #[default]
    NonSecure,
    /// 安全世界
    Secure,
    /// Realm 世界
    Realm,
}

/// 执行安全世界
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityWorld {
    /// 普通世界
    #[default]
    Normal,
    /// 安全世界
    Secure,
    /// Realm 世界
    Realm,
    /// Root 世界
    Root,
}

/// CPU 特权级（跨架构统一表示）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
    /// ARM EL0
    EL0,
    /// ARM EL1
    EL1,
    /// ARM EL2
    EL2,
    /// ARM EL3
    EL3,
    /// x86 Ring 0
    Ring0,
    /// x86 Ring 1
    Ring1,
    /// x86 Ring 2
    Ring2,
    /// x86 Ring 3
    Ring3,
    /// x86 VMX non-root
    RingMinus1,
    /// RISC-V U-mode
    U,
    /// RISC-V S-mode
    S,
    /// RISC-V H-mode
    H,
    /// RISC-V M-mode
    M,
}
