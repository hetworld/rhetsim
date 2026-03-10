//! Validator — Manifest 静态校验

use omnisim_core::error::OmniResult;
use crate::manifest::Manifest;

#[derive(Debug)]
pub struct ValidationReport {
    pub errors:   Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool { self.errors.is_empty() }
}

pub struct Validator;

impl Validator {
    pub fn new() -> Self { Self }

    pub fn validate(&self, manifest: &Manifest) -> OmniResult<ValidationReport> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if manifest.name.is_empty() {
            errors.push("manifest.name is empty".into());
        }
        if manifest.units.is_empty() {
            warnings.push("manifest.units is empty".into());
        }

        Ok(ValidationReport { errors, warnings })
    }
}
