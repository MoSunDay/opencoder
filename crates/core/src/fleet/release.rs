//! Runtime deployment configuration and compatibility contract.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const HANDOFF_PROTOCOL: u32 = 1;
pub const HANDOFF_DATA_FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConfig {
    pub release_id: String,
    pub state_dir: PathBuf,
    pub host_service: String,
    pub resource_service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatibleRange {
    pub min: u32,
    pub max: u32,
}

impl CompatibleRange {
    pub fn contains(&self, version: u32) -> bool {
        self.min <= version && version <= self.max
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseCompatibility {
    pub protocol: CompatibleRange,
    pub data_format: CompatibleRange,
}

impl ReleaseCompatibility {
    pub fn current() -> Self {
        Self {
            protocol: CompatibleRange {
                min: HANDOFF_PROTOCOL,
                max: HANDOFF_PROTOCOL,
            },
            data_format: CompatibleRange {
                min: HANDOFF_DATA_FORMAT,
                max: HANDOFF_DATA_FORMAT,
            },
        }
    }
    pub fn compatible(&self, peer: &Self) -> bool {
        self.protocol.contains(peer.protocol.min)
            && self.protocol.contains(peer.protocol.max)
            && self.data_format.contains(peer.data_format.min)
            && self.data_format.contains(peer.data_format.max)
    }
}
