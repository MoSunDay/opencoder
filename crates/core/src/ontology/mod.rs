//! Public capability descriptions. Execution and credentials stay in their native modules.
mod platform;
pub use platform::{platform_capabilities, platform_for_path, CLI_DOMAINS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Stable source-qualified identifier; independent of display names.
    pub key: String,
    pub source: String,
    pub source_id: String,
    pub kind: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub input_desc: String,
    pub output_desc: String,
    pub required_inputs: Vec<String>,
    pub required_outputs: Vec<String>,
    pub cli: Vec<String>,
    pub api: Vec<String>,
    /// Explicit source-qualified references, never inferred from prose.
    pub dependencies: Vec<String>,
    pub issues: Vec<String>,
}
