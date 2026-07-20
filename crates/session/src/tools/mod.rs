use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{AgentKind, ToolArc};
use serde_json::Value;

pub mod bash;
pub mod computer_use;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod ls;
pub mod read;
pub mod task;
pub mod web_read;
pub mod write;

#[cfg(feature = "browser")]
pub mod web_fetch;
#[cfg(feature = "browser")]
pub mod web_search;

pub fn registry() -> HashMap<String, ToolArc> {
    let all: Vec<ToolArc> = {
        // `mut` only used under the `browser` feature; allow when feature is off.
        #[allow(unused_mut)]
        let mut v: Vec<ToolArc> = vec![
            Arc::new(bash::BashTool) as ToolArc,
            Arc::new(read::ReadTool) as ToolArc,
            Arc::new(write::WriteTool) as ToolArc,
            Arc::new(edit::EditTool) as ToolArc,
            Arc::new(glob::GlobTool) as ToolArc,
            Arc::new(grep::GrepTool) as ToolArc,
            Arc::new(ls::ListTool) as ToolArc,
            Arc::new(task::TaskTool) as ToolArc,
            Arc::new(computer_use::ComputerUseTool) as ToolArc,
        ];
        // Browser tools are heavy (obscura + V8): only compiled with the
        // `browser` feature. Runtime visibility is additionally gated by
        // `capabilities.browser` in the runner's schema filter.
        #[cfg(feature = "browser")]
        v.extend([
            Arc::new(web_fetch::WebFetchTool) as ToolArc,
            Arc::new(web_search::WebSearchTool) as ToolArc,
        ]);
        v
    };
    all.into_iter().map(|t| (t.name().to_string(), t)).collect()
}

/// Project a (filtered) tool map into OpenAI function-calling JSON, applying the
/// per-tool schema sanitiser.
///
/// `kind` lets us special-case tools whose schema must change based on the owning
/// agent's kind, and `caps` adapts the `task` tool to the `tools_subagent`
/// capability. The `task` tool is rewritten via [`task::description_for`] /
/// [`task::parameters_for`] so:
/// - **plan mode** never reveals the `build` (full-write) subagent;
/// - a disabled `tools_subagent` capability never reveals the `tools` subagent.
///
/// This keeps the read-only / capability contracts at the *schema* layer, before
/// any runtime guard in `run_subagent` ever fires.
pub fn schema_for(
    tools: &HashMap<String, ToolArc>,
    kind: AgentKind,
    caps: &opencoder_core::CapabilitiesConfig,
) -> Vec<Value> {
    let tools_on = caps.tools_subagent_enabled();
    // Build (name, schema) pairs, then sort by name. A bare `.values().collect()`
    // would inherit `HashMap`'s randomized iteration order (Rust reseeds
    // `RandomState` per process), making the `tools` array in every ChatRequest
    // differ run-to-run: non-reproducible requests and order-sensitive tool
    // selection by the model. Sorting pins the order regardless of hash seed.
    let mut entries: Vec<(String, Value)> = tools
        .values()
        .map(|t| {
            let name = t.name();
            let (description, parameters) = if name == "task" {
                let plan = kind == AgentKind::Plan;
                (
                    task::description_for(plan, tools_on),
                    task::parameters_for(plan, tools_on),
                )
            } else {
                (t.description().to_string(), t.parameters())
            };
            let schema = serde_json::json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": opencoder_llm::schema::sanitize_tool_schema(&parameters),
                }
            });
            (name.to_string(), schema)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_only() -> HashMap<String, ToolArc> {
        let mut m = HashMap::new();
        let t = Arc::new(task::TaskTool) as ToolArc;
        m.insert(t.name().to_string(), t);
        m
    }

    fn task_schema(schemas: &[Value]) -> &Value {
        schemas
            .iter()
            .find(|v| v["function"]["name"] == "task")
            .expect("task schema present")
    }

    fn caps(tools_on: bool) -> opencoder_core::CapabilitiesConfig {
        opencoder_core::CapabilitiesConfig {
            tools_subagent: tools_on,
            ..Default::default()
        }
    }

    #[test]
    fn plan_mode_task_schema_omits_build() {
        let tools = task_only();
        let schemas = schema_for(&tools, AgentKind::Plan, &caps(true));
        let func = &task_schema(&schemas)["function"];

        let desc = func["description"].as_str().unwrap();
        assert!(
            !desc.contains("build"),
            "plan-mode task description must not mention 'build', got: {desc}"
        );
        assert!(
            desc.contains("explore"),
            "plan-mode task description must mention 'explore', got: {desc}"
        );

        let subagent_type_desc = func["parameters"]["properties"]["subagent_type"]["description"]
            .as_str()
            .unwrap();
        assert!(
            !subagent_type_desc.contains("build"),
            "plan-mode subagent_type description must not mention 'build', got: {subagent_type_desc}"
        );
        assert!(
            subagent_type_desc.contains("explore"),
            "plan-mode subagent_type description must mention 'explore', got: {subagent_type_desc}"
        );

        // Nothing build-related must leak anywhere in the parameters block.
        let params_str = func["parameters"].to_string();
        assert!(
            !params_str.contains("build"),
            "plan-mode task parameters must not contain 'build' anywhere, got: {params_str}"
        );
    }

    #[test]
    fn act_mode_task_schema_advertises_build() {
        // Regression guard: act mode must keep advertising the `build` subagent
        // so the model can delegate implementation work.
        let tools = task_only();
        let schemas = schema_for(&tools, AgentKind::Act, &caps(true));
        let func = &task_schema(&schemas)["function"];

        let desc = func["description"].as_str().unwrap();
        assert!(
            desc.contains("build"),
            "act-mode task description must mention 'build', got: {desc}"
        );
        let subagent_type_desc = func["parameters"]["properties"]["subagent_type"]["description"]
            .as_str()
            .unwrap();
        assert!(
            subagent_type_desc.contains("build"),
            "act-mode subagent_type description must mention 'build', got: {subagent_type_desc}"
        );
    }

    #[test]
    fn non_task_tools_unaffected_by_kind() {
        // Non-task tools must be unaffected by the kind parameter.
        let mut tools = HashMap::new();
        let r = Arc::new(read::ReadTool) as ToolArc;
        tools.insert(r.name().to_string(), r);
        let schemas = schema_for(&tools, AgentKind::Plan, &caps(false));
        let func = &schemas
            .iter()
            .find(|v| v["function"]["name"] == "read")
            .expect("read schema present")["function"];
        assert!(!func["description"].as_str().unwrap().is_empty());
    }

    #[test]
    fn schema_for_is_deterministically_ordered() {
        // The full tool registry is a `HashMap`, whose iteration order is
        // randomized per process (Rust reseeds `RandomState`). The `tools`
        // array sent in every ChatRequest must NOT depend on that hash seed,
        // otherwise requests are non-reproducible run-to-run (resumed sessions
        // would send tools in a different order than the original). Assert a
        // stable, sorted order. On the old unsorted code this assertion failed
        // ~randomly per process run.
        let tools = registry();
        for kind in [AgentKind::Act, AgentKind::Plan] {
            let schemas = schema_for(&tools, kind, &caps(false));
            let names: Vec<&str> = schemas
                .iter()
                .map(|v| v["function"]["name"].as_str().unwrap())
                .collect();
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(
                names, sorted,
                "tool schemas must be sorted by name for deterministic requests ({kind:?}); got {names:?}"
            );
        }
    }

    #[test]
    fn act_tools_off_schema_omits_tools() {
        // act + tools_subagent disabled: 'build' and 'explore' advertised, but
        // 'tools' must be hidden everywhere in the task schema.
        let tools = task_only();
        let schemas = schema_for(&tools, AgentKind::Act, &caps(false));
        let func = &task_schema(&schemas)["function"];
        let desc = func["description"].as_str().unwrap();
        assert!(
            !desc.contains("\"tools\""),
            "act+tools-off description must not mention 'tools', got: {desc}"
        );
        assert!(desc.contains("explore"), "must mention explore: {desc}");
        assert!(desc.contains("build"), "act must still advertise build: {desc}");
        let st_desc = func["parameters"]["properties"]["subagent_type"]["description"]
            .as_str()
            .unwrap();
        // Check for the quoted subagent type '"tools"' (not the bare word, which
        // legitimately appears in '(full tools)' when describing the build agent).
        assert!(
            !st_desc.contains("\"tools\""),
            "act+tools-off subagent_type must not list 'tools', got: {st_desc}"
        );
    }

    #[test]
    fn plan_tools_off_schema_omits_tools() {
        // plan + tools_subagent disabled: only 'explore' advertised — neither
        // 'build' (plan-hidden) nor 'tools' (capability-hidden) appear.
        let tools = task_only();
        let schemas = schema_for(&tools, AgentKind::Plan, &caps(false));
        let func = &task_schema(&schemas)["function"];
        let desc = func["description"].as_str().unwrap();
        assert!(
            !desc.contains("\"tools\""),
            "plan+tools-off description must not mention 'tools', got: {desc}"
        );
        assert!(
            !desc.contains("build"),
            "plan description must not mention 'build', got: {desc}"
        );
        assert!(desc.contains("explore"), "must mention explore: {desc}");
        let params_str = func["parameters"].to_string();
        assert!(
            !params_str.contains("tools"),
            "plan+tools-off parameters must not contain 'tools', got: {params_str}"
        );
        assert!(
            !params_str.contains("build"),
            "plan parameters must not contain 'build', got: {params_str}"
        );
    }
}
