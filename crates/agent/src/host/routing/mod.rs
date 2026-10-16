//! Runtime affinity follows persisted ownership, including pre-admission probes.
use super::Host;
use anyhow::{Context, Result};
use opencoder_core::fleet::ExecutionKind;
use serde_json::Value;

impl Host {
    pub(super) async fn inherited_runtime(
        &self,
        kind: ExecutionKind,
        input: &Value,
    ) -> Result<Option<String>> {
        let (parent, required) = if input["brain_workspace"] == true {
            (input["brain_layered"]["run_id"].as_str(), true)
        } else if kind == ExecutionKind::Brain && input["layered_request"]["parent"].is_object() {
            (input["layered_request"]["parent"]["run_id"].as_str(), true)
        } else if input["_brain"]["parent"].is_object() {
            (input["_brain"]["parent"]["run_id"].as_str(), true)
        } else {
            (input["brain_layered"]["run_id"].as_str(), false)
        };
        let Some(parent) = parent else {
            anyhow::ensure!(!required, "parent run identity missing");
            return Ok(None);
        };
        let owner = self.store.owner(parent).await?;
        if required {
            return Ok(Some(owner.context("parent runtime not found")?.runtime_id));
        }
        // Ordinary children may be independently placed on another physical
        // node. Local children retain the parent's release; remote ones use
        // the selected node's active runtime.
        Ok(owner.map(|owner| owner.runtime_id))
    }
}
