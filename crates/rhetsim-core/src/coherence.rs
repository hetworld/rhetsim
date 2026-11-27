//! 缓存一致性域（P2 接口占位）
use crate::error::OmniResult;
use crate::unit::UnitId;

/// 缓存一致性协议
#[derive(Debug, Clone, Copy)]
pub enum CoherenceProtocol {
    /// ARM AMBA ACE
    Ace,
    /// MOESI
    Moesi,
    /// CXL.cache 一致性
    CxlBi,
}

/// Snoop 操作
#[derive(Debug, Clone, Copy)]
pub enum SnoopOp {
    /// 读取共享副本
    ReadShared,
    /// 读取独占副本
    ReadUnique,
    /// 使副本失效
    Invalidate,
}

/// Snoop 响应
#[derive(Debug, Clone, Copy)]
pub enum SnoopResp {
    /// 命中干净副本
    Hit,
    /// 未命中
    Miss,
    /// 命中脏副本
    HitDirty,
}

/// 缓存维护操作
#[derive(Debug, Clone, Copy)]
pub enum CacheOp {
    /// 读取
    Read,
    /// 写入
    Write,
    /// 刷新
    Flush,
}

/// 一致性域参与者
pub trait CoherenceParticipant: Send + Sync {
    /// 处理一次 Snoop 请求
    fn on_snoop(&self, addr: u64, op: SnoopOp) -> OmniResult<SnoopResp>;
    /// 发起一次缓存操作
    fn request(&self, addr: u64, op: CacheOp) -> OmniResult<()>;
}

/// 缓存一致性域
pub struct CoherenceDomain {
    /// 域 ID
    pub id: String,
    /// 一致性协议
    pub protocol: CoherenceProtocol,
    /// 参与者；P2 之前为空，不产生实际效果
    pub members: Vec<UnitId>,
}
