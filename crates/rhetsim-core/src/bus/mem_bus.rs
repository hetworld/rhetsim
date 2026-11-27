//! MemBus — 地址总线

use crate::error::{OmniError, OmniResult};
use crate::mem::{MemAccessSize, MemAttrs, MemTarget};
use crate::security::{MemSecurity, SecurityWorld};
use std::sync::Arc;

/// 地址映射重叠策略
#[derive(Clone, Copy, Default)]
pub enum OverlapPolicy {
    /// 严格模式：重叠映射报错
    #[default]
    Strict,
    /// 影子模式：后注册的映射优先
    Shadow,
}

struct MemMapping {
    target: Arc<dyn MemTarget>,
    base: u64,
    size: u64,
    attrs: MemAttrs,
    visible_to: Option<Vec<u32>>,
}

/// 地址映射的只读描述
#[derive(Debug)]
pub struct MappingInfo {
    /// 映射基址
    pub base: u64,
    /// 映射大小
    pub size: u64,
    /// 目标类型描述
    pub target_kind: String,
    /// 访问属性
    pub attrs: MemAttrs,
    /// 可见的发起者 ID；None 表示对所有发起者可见
    pub visible_to: Option<Vec<u32>>,
}

/// 地址总线：MemTarget 到客户机地址的映射表
pub struct MemBus {
    /// 总线 ID
    pub id: String,
    mappings: Vec<MemMapping>,
    /// 预留的重叠策略；当前实现固定使用 Strict。
    #[allow(dead_code)]
    overlap_policy: OverlapPolicy,
}

impl MemBus {
    /// 创建空的地址总线
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            mappings: Vec::new(),
            overlap_policy: OverlapPolicy::Strict,
        }
    }

    /// 注册一个地址映射
    pub fn map(
        &mut self,
        target: Arc<dyn MemTarget>,
        base: u64,
        attrs: MemAttrs,
        visible_to: Option<Vec<u32>>,
    ) -> OmniResult<()> {
        let size = target.size();
        self.mappings.push(MemMapping {
            target,
            base,
            size,
            attrs,
            visible_to,
        });
        Ok(())
    }

    /// 移除指定基址的映射
    pub fn unmap(&mut self, base: u64) -> OmniResult<()> {
        self.mappings.retain(|m| m.base != base);
        Ok(())
    }

    /// 以指定发起者身份读取地址
    pub fn read(&self, addr: u64, size: MemAccessSize, initiator_id: u32) -> OmniResult<u64> {
        let m = self.find(addr, initiator_id)?;
        m.target.read(addr - m.base, size)
    }

    /// 以指定发起者身份写入地址
    pub fn write(
        &self,
        addr: u64,
        size: MemAccessSize,
        val: u64,
        initiator_id: u32,
    ) -> OmniResult<()> {
        let m = self.find(addr, initiator_id)?;
        m.target.write(addr - m.base, size, val)
    }

    /// 导出映射列表，可按发起者过滤
    pub fn dump_mappings(&self, initiator_id: Option<u32>) -> Vec<MappingInfo> {
        self.mappings
            .iter()
            .filter(|m| match (&m.visible_to, initiator_id) {
                (Some(v), Some(id)) => v.contains(&id),
                _ => true,
            })
            .map(|m| MappingInfo {
                base: m.base,
                size: m.size,
                target_kind: format!("target#{}", m.target.target_id()),
                attrs: m.attrs,
                visible_to: m.visible_to.clone(),
            })
            .collect()
    }

    fn find(&self, addr: u64, initiator_id: u32) -> OmniResult<&MemMapping> {
        self.mappings
            .iter()
            .filter(|m| addr >= m.base && addr < m.base + m.size)
            .rfind(|m| match &m.visible_to {
                Some(v) => v.contains(&initiator_id),
                None => true,
            })
            .ok_or(OmniError::AddressNotMapped(addr))
    }

    /// P2 安全检查占位（目前 no-op，保留接口供后续安全世界实现）
    #[allow(dead_code)]
    fn check_security(&self, _world: SecurityWorld, _security: MemSecurity) -> OmniResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_unmap() {
        let bus = MemBus::new("test");
        assert!(bus.dump_mappings(None).is_empty());
    }
}
