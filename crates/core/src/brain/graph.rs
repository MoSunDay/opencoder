//! Version 2 fixed graph contract. Definitions and runtime records are separate.
use super::{ActionSpec, ArtifactRef, ResourceUse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn twenty() -> u32 {
    20
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputSource {
    #[default]
    External,
    Routed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphInstance {
    pub id: String,
    pub description: String,
    pub capability_id: String,
    pub action: ActionSpec,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    #[serde(default)]
    pub resources: Vec<ResourceUse>,
    #[serde(default = "twenty")]
    pub max_visits: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphOutput {
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphRoute {
    pub id: String,
    pub description: String,
    pub outputs: Vec<String>,
    pub targets: Vec<RouteTarget>,
    #[serde(default)]
    pub exits: Vec<RouteExit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteTarget {
    pub instance: String,
    /// Target input ID -> connected output ID. Multiple sources use distinct inputs.
    pub bindings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteExit {
    pub id: String,
    pub description: String,
    pub deliverables: Vec<String>,
    #[serde(default = "yes")]
    pub require_completed: bool,
    #[serde(default = "yes")]
    pub require_verified: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    /// None is unknown; process termination is never evidence of success.
    #[serde(default)]
    pub passed: Option<bool>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProducedOutput {
    pub content: Value,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub completion: Assessment,
    #[serde(default)]
    pub verification: Assessment,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutputRecord {
    pub id: String,
    pub output: String,
    pub visit: String,
    pub round: u32,
    pub value: ProducedOutput,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GraphToken {
    pub visit: String,
    /// Fixed causal parents; joins consume live tokens, never latest-by-name output.
    pub parents: Vec<String>,
    pub inputs: BTreeMap<String, String>,
    #[serde(default)]
    pub scope: Vec<GraphScope>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphScope {
    pub fork: String,
    pub branch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RouteContext {
    pub receipt: String,
    pub route: String,
    pub description: String,
    pub outputs: Vec<OutputRecord>,
    pub candidates: Vec<RouteCandidate>,
    pub exits: Vec<RouteExit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RouteCandidate {
    pub instance: String,
    pub inputs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteDecision {
    pub receipt: String,
    pub reason: String,
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub exit: Option<String>,
    #[serde(default)]
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RouteReceipt {
    pub context: RouteContext,
    pub tokens: Vec<String>,
    pub decision: Option<RouteDecision>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GraphState {
    pub started: bool,
    pub tokens: BTreeMap<String, GraphToken>,
    #[serde(default)]
    pub visits: BTreeMap<String, GraphToken>,
    pub outputs: BTreeMap<String, OutputRecord>,
    pub routes: BTreeMap<String, RouteReceipt>,
    pub exits: Vec<String>,
}
