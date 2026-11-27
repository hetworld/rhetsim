//! PowerBus — 电源域管理总线

use crate::error::OmniResult;
use crate::unit::UnitId;
use std::sync::Arc;

/// 电源域状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// 正常上电
    On,
    /// 完全下电
    Off,
    /// 保持供电但暂停执行
    Retention,
}

/// 电源信号接收端
pub trait PowerSink: Send + Sync {
    /// 上电
    fn power_on(&self) -> OmniResult<()>;
    /// 下电
    fn power_off(&self) -> OmniResult<()>;
    /// 进入保持模式
    fn enter_retention(&self) -> OmniResult<()>;
    /// 查询当前电源状态
    fn power_state(&self) -> PowerState;
}

/// 一条电源域连接
pub struct PowerConnection {
    /// 电源控制器 Unit
    pub controller: UnitId,
    /// 目标 Unit
    pub target: UnitId,
    /// 电源域名
    pub domain: String,
}

/// 电源域管理总线
pub struct PowerBus {
    /// 总线 ID
    pub id: String,
    /// 已登记的电源连接
    pub connections: Vec<PowerConnection>,
    sinks: Vec<(UnitId, Arc<dyn PowerSink>)>,
}

impl PowerBus {
    /// 创建空的电源总线
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            connections: Vec::new(),
            sinks: Vec::new(),
        }
    }

    /// 注册一个电源接收端
    pub fn register_sink(&mut self, unit_id: UnitId, sink: Arc<dyn PowerSink>) {
        self.sinks.push((unit_id, sink));
    }

    /// 给目标 Unit 上电
    pub fn power_on(&self, target: &UnitId) -> OmniResult<()> {
        self.find(target)?.power_on()
    }

    /// 给目标 Unit 下电
    pub fn power_off(&self, target: &UnitId) -> OmniResult<()> {
        self.find(target)?.power_off()
    }

    /// 让目标 Unit 进入保持模式
    pub fn enter_retention(&self, target: &UnitId) -> OmniResult<()> {
        self.find(target)?.enter_retention()
    }

    /// 查询目标 Unit 的电源状态
    pub fn state(&self, target: &UnitId) -> OmniResult<PowerState> {
        Ok(self.find(target)?.power_state())
    }

    fn find(&self, target: &UnitId) -> OmniResult<&dyn PowerSink> {
        self.sinks
            .iter()
            .find(|(id, _)| id == target)
            .map(|(_, s)| s.as_ref())
            .ok_or_else(|| crate::error::OmniError::UnitNotFound(target.to_string()))
    }
}
