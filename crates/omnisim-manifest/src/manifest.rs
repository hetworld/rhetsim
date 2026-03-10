//! Manifest / UnitDesc / BusDesc / BootDesc — 平台声明式描述

use serde::{Deserialize, Serialize};
use omnisim_core::backend::execution::BackendKind;
use omnisim_core::backend::timing::TimingKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name:      String,
    #[serde(default)]
    pub templates: Vec<UnitDesc>,
    pub units:     Vec<UnitDesc>,
    #[serde(default)]
    pub buses:     Vec<BusDesc>,
    pub boot:      BootDesc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDesc {
    pub id:       String,
    pub kind:     String,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub backend:  BackendKind,
    #[serde(default)]
    pub timing:   TimingKind,
    #[serde(default)]
    pub config:   serde_json::Value,
    #[serde(default)]
    pub sub_units: Vec<UnitDesc>,
    #[serde(default)]
    pub count:    Option<CountSpec>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CountSpec {
    Fixed(u32),
    Range { from: u32, to: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusDesc {
    Mem {
        id:         String,
        #[serde(default)]
        overlap:    bool,
        initiators: Vec<String>,
        targets:    Vec<MemTargetDesc>,
    },
    Irq {
        id:          String,
        connections: Vec<IrqConnectionDesc>,
    },
    Data {
        id:        String,
        producers: Vec<String>,
        consumers: Vec<String>,
    },
    Sync {
        id:              String,
        participants:    Vec<String>,
        #[serde(default = "default_quantum")]
        time_quantum_ns: u64,
    },
    Pcie {
        id:           String,
        root_complex: String,
        endpoints:    Vec<PcieEndpointDesc>,
    },
    Reset {
        id:          String,
        connections: Vec<ResetConnectionDesc>,
    },
    Power {
        id:          String,
        connections: Vec<PowerConnectionDesc>,
    },
    Interconnect {
        id:        String,
        addr_mode: String,
        nodes:     Vec<String>,
    },
}

fn default_quantum() -> u64 { 1_000_000 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemTargetDesc {
    pub unit:       String,
    pub base:       u64,
    pub size:       Option<u64>,
    pub visible_to: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrqConnectionDesc {
    pub from: String,
    pub to:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PcieEndpointDesc {
    pub unit:     String,
    pub bus:      u8,
    pub device:   u8,
    pub function: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetConnectionDesc {
    pub from: String,
    pub to:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerConnectionDesc {
    pub controller: String,
    pub target:     String,
    pub domain:     String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BootDesc {
    #[serde(default)]
    pub kernel:   Option<String>,
    #[serde(default)]
    pub initrd:   Option<String>,
    #[serde(default)]
    pub dtb:      Option<String>,
    #[serde(default)]
    pub append:   Option<String>,
    #[serde(default)]
    pub sequence: Vec<serde_json::Value>,
}
