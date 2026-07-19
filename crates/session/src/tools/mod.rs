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
/// agent's kind. In **plan mode** the `task` tool is rewritten so the model is
/// never told that the `build` (full-write) subagent exists — see
/// [`task::description_plan`] / [`task::parameters_plan`]. This keeps the
/// plan-mode read-only contract at the *schema* layer, before any runtime guard
/// ever fires.
pub fn schema_for(tools: &HashMap<String, ToolArc>, kind: AgentKind) -> Vec<Value> {
    // Build (name, schema) pairs, then sort by name. A bare `.values().collect()`
    // would inherit `HashMap`'s randomized iteration order (Rust reseeds
    // `RandomState` per process), making the `tools` array in every ChatRequest
    // differ run-to-run: non-reproducible requests and order-sensitive tool
    // selection by the model. Sorting pins the order regardless of hash seed.
    let mut entries: Vec<(String, Value)> = tools
        .values()
        .map(|t| {
            let name = t.name();
            let (description, parameters) = if kind == AgentKind::Plan && name == "task" {
                (task::description_plan(), task::parameters_plan())
            } else {
                (t.description(), t.parameters())
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

    #[test]
    fn plan_mode_task_schema_omits_build() {
        let tools = task_only();
        let schemas = schema_for(&tools, AgentKind::Plan);
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
        let schemas = schema_for(&tools, AgentKind::Act);
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
        let schemas = schema_for(&tools, AgentKind::Plan);
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
            let schemas = schema_for(&tools, kind);
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
}
