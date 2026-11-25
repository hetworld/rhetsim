//! 安全执行接口（P0 预留，P2 实施）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemSecurity { #[default] NonSecure, Secure, Realm }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityWorld { #[default] Normal, Secure, Realm, Root }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
    EL0, EL1, EL2, EL3,
    Ring0, Ring1, Ring2, Ring3, RingMinus1,
    U, S, H, M,
}
