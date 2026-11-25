//! DataBus — 高带宽数据流总线

use std::sync::Arc;
use std::collections::HashMap;
use crate::error::OmniResult;
use crate::dataflow::{DataEndpoint, DataDesc, DataToken, DataResult};
use crate::unit::UnitId;

pub struct DataBus {
    pub id:        String,
    producers:     Vec<(UnitId, Arc<dyn DataEndpoint>)>,
    consumers:     Vec<(UnitId, Arc<dyn DataEndpoint>)>,
}

impl DataBus {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), producers: Vec::new(), consumers: Vec::new() }
    }

    pub fn add_producer(&mut self, unit_id: UnitId, ep: Arc<dyn DataEndpoint>) {
        self.producers.push((unit_id, ep));
    }

    pub fn add_consumer(&mut self, unit_id: UnitId, ep: Arc<dyn DataEndpoint>) {
        self.consumers.push((unit_id, ep));
    }

    pub fn submit(&self, producer_id: &UnitId, desc: DataDesc) -> OmniResult<DataToken> {
        let ep = self.producers.iter()
            .find(|(id, _)| id == producer_id)
            .map(|(_, ep)| ep)
            .ok_or_else(|| crate::error::OmniError::UnitNotFound(producer_id.to_string()))?;
        ep.produce(desc)
    }
}
