//! Cross-name comparison requires the actual current agent resource bindings,
//! not just the abbreviated prompt/tools fields returned by the catalog.
use serde_json::{json, Value};

pub(super) fn identity(value: &Value) -> Option<Value> {
    if value["kind"] != "agent" || value.get("unavailable_reason").is_some() {
        return None;
    }
    let name = value["target"].as_str()?;
    let meta = opencoder_core::agent::read_agent_meta(name)?;
    // Validate every referenced resource and reject unsupported symlinks. Shared
    // binding names remain in the identity: independently mutable pools differ.
    opencoder_core::brain::resources::agent_manifest(name).ok()?;
    let mut definition = value["definition"].clone();
    definition.as_object_mut()?.remove("name");
    Some(
        json!({"definition":definition,"harness":meta.harness,"profile":meta.harness_profile,"resources":meta.current}),
    )
}
