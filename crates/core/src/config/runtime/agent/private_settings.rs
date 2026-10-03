//! Configuration overlays preserve launch fields and whole profile revisions.
use crate::harness::{CodexSettings, RuntimeSettings};
use serde_json::Value;

pub(in crate::config) fn merge_codex(
    current: &Option<CodexSettings>,
    patch: &Value,
) -> Option<Option<CodexSettings>> {
    super::super::merge::merge_fields(current, patch)
}

pub(in crate::config) fn merge_runtime(
    current: &RuntimeSettings,
    patch: &Value,
) -> Option<RuntimeSettings> {
    let patch = serde_json::from_value(patch.clone()).ok()?;
    // A profile revision is one snapshot; never mix its settings with an older one.
    Some(current.with_server(&patch))
}
