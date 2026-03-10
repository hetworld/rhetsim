//! Expander — 模板展开 / count 数量展开

use omnisim_core::error::OmniResult;
use crate::manifest::{Manifest, UnitDesc, CountSpec};

pub struct Expander;

impl Expander {
    pub fn new() -> Self { Self }

    pub fn expand(&self, manifest: &Manifest) -> OmniResult<Manifest> {
        let mut expanded = manifest.clone();
        expanded.units = manifest.units.iter()
            .flat_map(|u| self.expand_unit(u))
            .collect();
        Ok(expanded)
    }

    fn expand_unit(&self, desc: &UnitDesc) -> Vec<UnitDesc> {
        match &desc.count {
            None => vec![desc.clone()],
            Some(CountSpec::Fixed(n)) => (0..*n).map(|i| {
                let mut d = desc.clone();
                d.id = format!("{}[{}]", desc.id, i);
                d.count = None;
                d
            }).collect(),
            Some(CountSpec::Range { from, to }) => (*from..=*to).map(|i| {
                let mut d = desc.clone();
                d.id = format!("{}[{}]", desc.id, i);
                d.count = None;
                d
            }).collect(),
        }
    }
}
