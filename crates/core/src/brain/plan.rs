use crate::fleet::ExecutionKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn one() -> u32 {
    1
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanRef {
    pub id: String,
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OntologyPlan {
    #[serde(default = "one")]
    pub schema_version: u32,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, InputPort>,
    pub steps: Vec<StepTemplate>,
    pub deliverables: BTreeMap<String, Deliverable>,
    #[serde(default)]
    pub references: Vec<PlanRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<ActionFlow>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActionFlow {
    pub entry: String,
    #[serde(default = "twenty")]
    pub max_visits_per_action: u32,
    pub transitions: Vec<ActionTransition>,
}
fn twenty() -> u32 {
    20
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActionTransition {
    pub from: String,
    pub to: Option<String>,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InputPort {
    pub description: String,
    pub schema: DataSchema,
    #[serde(default = "yes")]
    pub required: bool,
}

/// Deliberately bounded JSON-schema subset. Semantic types require exact
/// agreement; conversion must be an explicit capability step.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DataSchema {
    #[serde(rename = "type")]
    pub kind: DataType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, DataSchema>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<DataSchema>>,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default, rename = "enum", skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
    Null,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StepTemplate {
    pub id: String,
    pub label: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    pub action: ActionSpec,
    #[serde(default)]
    pub inputs: BTreeMap<String, StepInput>,
    pub output: DataSchema,
    pub acceptance: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default, rename = "foreach", skip_serializing_if = "Option::is_none")]
    pub expansion: Option<Expansion>,
    #[serde(default)]
    pub resources: Vec<ResourceUse>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StepInput {
    pub schema: DataSchema,
    pub binding: Binding,
    #[serde(default = "yes")]
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum Binding {
    Literal {
        value: Value,
    },
    Input {
        name: String,
        #[serde(default)]
        path: String,
    },
    Output {
        step: String,
        #[serde(default)]
        path: String,
        #[serde(default)]
        collect: bool,
    },
    Item {
        #[serde(default)]
        path: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub value: Binding,
    pub equals: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Expansion {
    pub items: Binding,
    /// JSON pointer into each item. Empty selects the entire scalar item.
    pub key: String,
    #[serde(default = "yes")]
    pub allow_empty: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    #[serde(default)]
    pub agent_manifests: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<Value>,
    pub kind: ExecutionKind,
    pub target: String,
    pub prompt: String,
    /// Pinned execution definition. Agent snapshots include card/resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<Value>,
    #[serde(default)]
    pub output_mode: OutputMode,
    #[serde(default)]
    pub output_pointer: String,
    #[serde(default = "one")]
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceUse {
    pub key: String,
    pub mode: AccessMode,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    Read,
    Write,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Deliverable {
    pub description: String,
    pub source: Binding,
    pub schema: DataSchema,
    /// Optional machine-verifiable expected value (for example true from
    /// the final verification step). Absence means schema + step acceptance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlanVersion {
    pub id: String,
    pub version: u64,
    pub plan: OntologyPlan,
    pub changelog: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub confidence: Confidence,
    pub created_at: i64,
    #[serde(default)]
    pub author: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Confidence {
    #[serde(default)]
    pub level: ConfidenceLevel,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    #[default]
    Unverified,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlanDefinition {
    pub id: String,
    pub title: String,
    pub latest_version: u64,
    pub stable_version: Option<u64>,
    pub updated_at: i64,
}
