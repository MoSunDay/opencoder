//! Milestone objectives and allowed capabilities.
use crate::fleet::ExecutionKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn max_rounds() -> u32 {
    5
}
fn default_attempts() -> u32 {
    2
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredTodoRef {
    pub id: String,
}

/// Retained only for reading historical schema 4 plans.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredRetry {
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
}
impl Default for LayeredRetry {
    fn default() -> Self {
        Self {
            max_attempts: default_attempts(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredNode {
    #[serde(default)]
    pub layer: u32,
    #[serde(default)]
    pub layer_id: String,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub success_criteria: String,
    #[serde(default)]
    pub capability_ids: Vec<String>,
    pub node_id: String,
    pub title: String,
    /// One execution capability in schema 7; historical schema 4 also uses this field.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub capability_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<LayeredRetry>,
}
impl LayeredNode {
    pub fn capability_refs(&self) -> Vec<&str> {
        if self.capability_id.is_empty() {
            self.capability_ids.iter().map(String::as_str).collect()
        } else {
            vec![&self.capability_id]
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredMilestone {
    pub layer_id: String,
    pub title: String,
    #[serde(default)]
    pub task: String,
    pub objective: String,
    pub success_criteria: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredTransition {
    pub from: String,
    pub to: String,
    pub condition: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub condition: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredPlan {
    pub schema_version: u32,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo: Option<LayeredTodoRef>,
    pub nodes: Vec<LayeredNode>,
    #[serde(default)]
    pub layers: Vec<LayeredMilestone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<LayeredTransition>,
    #[serde(default)]
    pub edges: Vec<LayeredEdge>,
    #[serde(default = "max_rounds")]
    pub max_rounds: u32,
}
impl LayeredPlan {
    pub fn node(&self, node_id: &str) -> Option<&LayeredNode> {
        self.nodes.iter().find(|node| node.node_id == node_id)
    }

    /// Persist paths understood by retained schema-7 runtimes. The current
    /// Brain ignores these paths and decides from evidence and capabilities.
    /// Existing paths stay intact so retries of historical versions are exact.
    pub fn with_rollback_paths(mut self) -> Self {
        if self.schema_version != 7 || !self.transitions.is_empty() {
            return self;
        }
        for (index, layer) in self.layers.iter().enumerate() {
            if let Some(next) = self.layers.get(index + 1) {
                self.transitions.push(LayeredTransition {
                    from: layer.layer_id.clone(),
                    to: next.layer_id.clone(),
                    condition: "Current milestone meets its success criteria".into(),
                });
            }
            for target in self.layers.iter().take(index + 1) {
                self.transitions.push(LayeredTransition {
                    from: layer.layer_id.clone(),
                    to: target.layer_id.clone(),
                    condition: "Evidence requires rework in this previously executed milestone"
                        .into(),
                });
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_paths_cover_every_executed_layer_without_changing_old_versions() {
        let mut plan: LayeredPlan = serde_json::from_value(serde_json::json!({
            "schema_version": 7, "title": "work", "objective": "finish",
            "nodes": [], "layers": [
                {"layer_id":"a","title":"A","objective":"A","success_criteria":"done"},
                {"layer_id":"b","title":"B","objective":"B","success_criteria":"done"},
                {"layer_id":"c","title":"C","objective":"C","success_criteria":"done"}
            ]
        }))
        .unwrap();
        let generated = plan.clone().with_rollback_paths();
        let paths = generated
            .transitions
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                ("a", "b"),
                ("a", "a"),
                ("b", "c"),
                ("b", "a"),
                ("b", "b"),
                ("c", "a"),
                ("c", "b"),
                ("c", "c"),
            ]
        );
        assert_eq!(generated.clone().with_rollback_paths(), generated);
        plan.transitions.push(LayeredTransition {
            from: "a".into(),
            to: "b".into(),
            condition: "historical rule".into(),
        });
        assert_eq!(plan.clone().with_rollback_paths(), plan);
    }
}

/// Frozen origin of a run, kept for the workbench even if the plan changes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredOrigin {
    pub plan_id: String,
    pub version: u64,
}

/// Nesting identity: the child run reports back to the parent operation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredParent {
    pub run_id: String,
    pub operation_id: String,
    pub node_id: String,
    pub layer: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredRequest {
    pub schema_version: u32,
    pub plan: LayeredPlan,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, crate::brain::ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<LayeredOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<LayeredParent>,
    /// Nesting depth: roots are 0, a nested plan adds one per level.
    #[serde(default)]
    pub depth: u32,
}

/// Bounded project-todo projection carried into the layer context.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayeredTodoSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub draft: String,
}

/// A capability synthesized for a plan definition, so a plan can nest a plan.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayeredPlanCapability {
    pub plan_id: String,
    pub version: u64,
    pub title: String,
    pub objective: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub capability_id: String,
    pub kind: ExecutionKind,
    pub node_count: usize,
    pub layer_count: usize,
}
