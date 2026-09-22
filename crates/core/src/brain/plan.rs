use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanRef {
    pub id: String,
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlanVersion<P = super::layered::LayeredPlan> {
    pub id: String,
    pub version: u64,
    pub plan: P,
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
