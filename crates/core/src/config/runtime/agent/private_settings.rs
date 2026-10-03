//! Configuration overlays preserve launch fields and whole profile revisions.
use crate::harness::{CodexSettings, RuntimeSettings};
use serde_json::Value;

pub(in crate::config) fn merge_codex(
    current: &Option<CodexSettings>,
    patch: &Value,
) -> Option<Option<CodexSettings>> {
    if patch.is_null() {
        return Some(None);
    }
    let mut merged = serde_json::to_value(current).ok()?;
    super::super::merge::merge_json(&mut merged, patch);
    serde_json::from_value::<CodexSettings>(merged)
        .ok()
        .map(Some)
}

pub(in crate::config) fn merge_runtime(
    current: &RuntimeSettings,
    patch: &Value,
) -> Option<RuntimeSettings> {
    let patch = serde_json::from_value(patch.clone()).ok()?;
    // A profile revision is one snapshot; never mix its settings with an older one.
    Some(current.with_server(&patch))
}
