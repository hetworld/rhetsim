//! MemBus — 地址总线

use std::sync::Arc;
use crate::error::{OmniError, OmniResult};
use crate::mem::{MemTarget, MemAttrs, MemAccessSize};
use crate::security::{MemSecurity, SecurityWorld};

#[derive(Clone, Copy, Default)]
pub enum OverlapPolicy { #[default] Strict, Shadow }

struct MemMapping {
    target:      Arc<dyn MemTarget>,
    base:        u64,
    size:        u64,
    attrs:       MemAttrs,
    visible_to:  Option<Vec<u32>>,
}

#[derive(Debug)]
pub struct MappingInfo {
    pub base:        u64,
    pub size:        u64,
    pub target_kind: String,
    pub attrs:       MemAttrs,
    pub visible_to:  Option<Vec<u32>>,
}

pub struct MemBus {
    pub id:             String,
    mappings:           Vec<MemMapping>,
    overlap_policy:     OverlapPolicy,
}

impl MemBus {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), mappings: Vec::new(), overlap_policy: OverlapPolicy::Strict }
    }

    pub fn map(
        &mut self,
        target:     Arc<dyn MemTarget>,
        base:       u64,
        attrs:      MemAttrs,
        visible_to: Option<Vec<u32>>,
    ) -> OmniResult<()> {
        let size = target.size();
        self.mappings.push(MemMapping { target, base, size, attrs, visible_to });
        Ok(())
    }

    pub fn unmap(&mut self, base: u64) -> OmniResult<()> {
        self.mappings.retain(|m| m.base != base);
        Ok(())
    }

    pub fn read(&self, addr: u64, size: MemAccessSize, initiator_id: u32) -> OmniResult<u64> {
        let m = self.find(addr, initiator_id)?;
        m.target.read(addr - m.base, size)
    }

    pub fn write(&self, addr: u64, size: MemAccessSize, val: u64, initiator_id: u32)
        -> OmniResult<()>
    {
        let m = self.find(addr, initiator_id)?;
        m.target.write(addr - m.base, size, val)
    }

    pub fn dump_mappings(&self, initiator_id: Option<u32>) -> Vec<MappingInfo> {
        self.mappings.iter()
            .filter(|m| match (&m.visible_to, initiator_id) {
                (Some(v), Some(id)) => v.contains(&id),
                _ => true,
            })
            .map(|m| MappingInfo {
                base:        m.base,
                size:        m.size,
                target_kind: format!("target#{}", m.target.target_id()),
                attrs:       m.attrs,
                visible_to:  m.visible_to.clone(),
            })
            .collect()
    }

    fn find(&self, addr: u64, initiator_id: u32) -> OmniResult<&MemMapping> {
        self.mappings.iter()
            .filter(|m| addr >= m.base && addr < m.base + m.size)
            .filter(|m| match &m.visible_to {
                Some(v) => v.contains(&initiator_id),
                None    => true,
            })
            .last()
            .ok_or(OmniError::AddressNotMapped(addr))
    }

    /// P2 安全检查占位（目前 no-op）
    fn check_security(&self, _world: SecurityWorld, _security: MemSecurity) -> OmniResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_unmap() {
        let mut bus = MemBus::new("test");
        assert!(bus.dump_mappings(None).is_empty());
    }
}
