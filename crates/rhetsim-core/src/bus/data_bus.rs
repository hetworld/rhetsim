//! DataBus — 高带宽数据流总线

use crate::dataflow::{DataDesc, DataEndpoint, DataToken};
use crate::error::OmniResult;
use crate::unit::UnitId;
use std::sync::Arc;

/// 高带宽数据流总线
pub struct DataBus {
    /// 总线 ID
    pub id: String,
    producers: Vec<(UnitId, Arc<dyn DataEndpoint>)>,
    consumers: Vec<(UnitId, Arc<dyn DataEndpoint>)>,
}

impl DataBus {
    /// 创建空的数据总线
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            producers: Vec::new(),
            consumers: Vec::new(),
        }
    }

    /// 注册生产者端点
    pub fn add_producer(&mut self, unit_id: UnitId, ep: Arc<dyn DataEndpoint>) {
        self.producers.push((unit_id, ep));
    }

    /// 注册消费者端点
    pub fn add_consumer(&mut self, unit_id: UnitId, ep: Arc<dyn DataEndpoint>) {
        self.consumers.push((unit_id, ep));
    }

    /// 由指定生产者提交一次数据传输
    pub fn submit(&self, producer_id: &UnitId, desc: DataDesc) -> OmniResult<DataToken> {
        let ep = self
            .producers
            .iter()
            .find(|(id, _)| id == producer_id)
            .map(|(_, ep)| ep)
            .ok_or_else(|| crate::error::OmniError::UnitNotFound(producer_id.to_string()))?;
        ep.produce(desc)
    }
}
