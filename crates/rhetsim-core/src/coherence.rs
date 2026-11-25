//! 缓存一致性域（P2 接口占位）
use crate::error::OmniResult;
use crate::unit::UnitId;

#[derive(Debug, Clone, Copy)]
pub enum CoherenceProtocol { Ace, Moesi, CxlBi }

#[derive(Debug, Clone, Copy)]
pub enum SnoopOp { ReadShared, ReadUnique, Invalidate }

#[derive(Debug, Clone, Copy)]
pub enum SnoopResp { Hit, Miss, HitDirty }

#[derive(Debug, Clone, Copy)]
pub enum CacheOp { Read, Write, Flush }

pub trait CoherenceParticipant: Send + Sync {
    fn on_snoop(&self, addr: u64, op: SnoopOp) -> OmniResult<SnoopResp>;
    fn request(&self, addr: u64, op: CacheOp)  -> OmniResult<()>;
}

pub struct CoherenceDomain {
    pub id:       String,
    pub protocol: CoherenceProtocol,
    /// P2 之前 members 为空，不产生实际效果
    pub members:  Vec<UnitId>,
}
