use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{AgentKind, ToolArc};
use serde_json::Value;

pub mod bash;
pub mod bg;
pub mod edit;
pub mod image_data;
pub mod latent;
pub mod ls;
pub mod question;
pub mod read;
pub mod search;
pub mod ssh_pty;
pub mod task;
pub mod view_image;

pub fn registry() -> HashMap<String, ToolArc> {
    let all: Vec<ToolArc> = vec![
        Arc::new(bash::BashTool) as ToolArc,
        Arc::new(read::ReadTool) as ToolArc,
        Arc::new(view_image::ViewImageTool) as ToolArc,
        Arc::new(edit::EditTool) as ToolArc,
        Arc::new(search::SearchTool) as ToolArc,
        Arc::new(ls::ListTool) as ToolArc,
        // Placeholder hub: registry() callers only project the schema /
        // estimate tokens. The runner's `build_full_registry` rebinds this
        // entry to the session's shared hub so answers actually flow.
        Arc::new(question::QuestionTool::new(question::QuestionHub::new())) as ToolArc,
        Arc::new(task::TaskTool) as ToolArc,
        Arc::new(ssh_pty::SshPtyTool) as ToolArc,
    ];
    all.into_iter().map(|t| (t.name().to_string(), t)).collect()
}

/// Whether the `build` subagent must be hidden from every prompt surface:
/// **plan mode** always (read-only contract; the kind was serialized as
/// `sandbox` during the sandbox-mode interlude), plus any agent while the
/// **task-plan skill** is active (plan-only turns are not advertised
/// implementation delegation). Thin projection of
/// [`opencoder_core::build_delegation_hidden`]; shared by the runner's
/// schema build, the system prompt, and the token estimator so all
/// surfaces agree.
pub fn hide_build_subagent(kind: AgentKind, skill_body: Option<&str>) -> bool {
    opencoder_core::build_delegation_hidden(kind, latent::task_plan_active(skill_body))
}

/// Project a (filtered) tool map into OpenAI function-calling JSON, applying the
/// per-tool schema sanitiser.
///
/// `hide_build` lets us special-case the `task` tool: it is rewritten via
/// [`task::description_for`] / [`task::parameters_for`] so the `build`
/// (full-write) subagent is never revealed when [`hide_build_subagent`]
/// holds. This keeps the contract at the *schema* layer, before any runtime
/// guard in `run_subagent` ever fires.
pub fn schema_for(tools: &HashMap<String, ToolArc>, hide_build: bool) -> Vec<Value> {
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
                (
                    task::description_for(hide_build),
                    task::parameters_for(hide_build),
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

/// Estimate the token cost of the tool-definition JSON that would be sent in a
/// `ChatRequest` for the given agent + skill. This closes the gap between the
/// local estimate (used by compaction + TUI display) and the real
/// `prompt_tokens` reported by the provider, which includes the full tool
/// schema array.
///
/// The filtering logic mirrors `runner::llm_call::run_one_llm_call` exactly
/// (agent allowlist ∧ latent-gating, with the plan `question` exemption),
/// so the estimate matches what the provider actually receives.
pub fn estimate_tool_schema_tokens(
    agent: &opencoder_core::Agent,
    skill_body: Option<&str>,
    registry: &HashMap<String, ToolArc>,
) -> usize {
    let unlocked = latent::unlocked_from_body(skill_body);
    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| latent::is_visible(name.as_str(), agent, &unlocked))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let schemas = schema_for(&allowed, hide_build_subagent(agent.kind, skill_body));
    let json = serde_json::to_string(&schemas).unwrap_or_default();
    opencoder_llm::estimate(&json)
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

    /// A `body_with_source`-shaped task-plan skill body (the real activation
    /// shape; the exact Source line is what `task_plan_active` resolves).
    fn plan_skill_body() -> Option<&'static str> {
        Some("> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\nplan body")
    }

    #[test]
    fn hide_build_subagent_matrix() {
        let plan = plan_skill_body();
        let lookalike = Some("> Source: /skills/my-task-plan/SKILL.md\n\nbody");
        // Plan always hides; act hides only while task-plan is active.
        assert!(hide_build_subagent(AgentKind::Plan, None));
        assert!(hide_build_subagent(AgentKind::Plan, plan));
        assert!(!hide_build_subagent(AgentKind::Act, None));
        assert!(hide_build_subagent(AgentKind::Act, plan));
        assert!(!hide_build_subagent(AgentKind::Act, lookalike));
    }

    #[test]
    fn act_task_plan_task_schema_omits_build() {
        let tools = task_only();
        let schemas = schema_for(
            &tools,
            hide_build_subagent(AgentKind::Act, plan_skill_body()),
        );
        let func = &task_schema(&schemas)["function"];

        let desc = func["description"].as_str().unwrap();
        assert!(
            !desc.contains("build"),
            "task-plan act task description must not mention 'build', got: {desc}"
        );
        assert!(
            desc.contains("explore"),
            "task-plan act task description must mention 'explore', got: {desc}"
        );
        let params_str = func["parameters"].to_string();
        assert!(
            !params_str.contains("build"),
            "task-plan act task parameters must not contain 'build', got: {params_str}"
        );
    }

    #[test]
    fn act_without_task_plan_task_schema_keeps_build() {
        let tools = task_only();
        let schemas = schema_for(&tools, hide_build_subagent(AgentKind::Act, None));
        let desc = task_schema(&schemas)["function"]["description"]
            .as_str()
            .unwrap();
        assert!(
            desc.contains("\"build\""),
            "plain act task description must still advertise build, got: {desc}"
        );
    }

    #[test]
    fn plan_mode_task_schema_omits_build() {
        let tools = task_only();
        let schemas = schema_for(&tools, true);
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
        let schemas = schema_for(&tools, false);
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
        let schemas = schema_for(&tools, true);
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
            let hide_build = kind == AgentKind::Plan;
            let schemas = schema_for(&tools, hide_build);
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

    /// The `question` tool is latent and gated by the task-plan skill:
    /// the plan agent always sees it (plan-kind parity exemption), an act
    /// agent sees it only once a skill body whose
    /// first 500 chars name the skill unlocks it, and non-primary agents never
    /// do. The schema itself stays cheap (<200 tokens).
    #[test]
    fn question_schema_is_plan_only_and_compact() {
        let reg = registry();
        let plan = opencoder_core::resolve_agent("plan").unwrap();
        let act = opencoder_core::resolve_agent("act").unwrap();
        let command = opencoder_core::resolve_agent("command").unwrap();

        // Plan: question is visible with NO skill at all.
        let plan_tokens = estimate_tool_schema_tokens(&plan, None, &reg);

        // Act without a skill: question absent. With a task-plan body (the
        // skill name inside the 500-char prefix window): present. A `review`
        // body must NOT unlock it (question is task-plan-only).
        let act_tokens = estimate_tool_schema_tokens(&act, None, &reg);
        let plan_body = Some("# task-plan\n\n## Overview\n\nplan the work; ask via question");
        let review_body = Some("# review\n\nevidence-driven check; use question when blocked");
        let act_unlocked_plan = estimate_tool_schema_tokens(&act, plan_body, &reg);
        let act_unlocked_review = estimate_tool_schema_tokens(&act, review_body, &reg);

        // Isolate the question schema's own cost (plan always includes it).
        let mut without = reg.clone();
        without.remove("question");
        let plan_without = estimate_tool_schema_tokens(&plan, None, &without);
        let cost = plan_tokens - plan_without;
        assert!(cost > 0, "plan agent must see the question schema");
        assert_eq!(
            act_unlocked_plan - estimate_tool_schema_tokens(&act, plan_body, &without),
            cost,
            "act agent with an unlocked skill sees question at the same cost"
        );
        assert!(
            act_unlocked_plan > act_tokens,
            "a task-plan body must unlock the question schema for act: {act_tokens} -> {act_unlocked_plan}"
        );
        assert_eq!(
            act_unlocked_review, act_tokens,
            "a review body must NOT unlock question for act (task-plan-only)"
        );
        assert!(
            plan_tokens > act_tokens,
            "plan must carry the question schema that a skill-less act lacks: {act_tokens} vs {plan_tokens}"
        );
        assert_eq!(
            estimate_tool_schema_tokens(&command, None, &reg),
            estimate_tool_schema_tokens(&command, None, &without),
            "command agent must NOT see the question schema"
        );

        assert!(cost > 0, "the question schema must cost something");
        assert!(
            cost < 200,
            "question schema should stay compact (<200 tokens), got {cost}"
        );
    }

    #[test]
    fn estimate_tool_schema_tokens_is_nontrivial() {
        let agent = opencoder_core::resolve_agent("act").expect("act agent");
        let reg = registry();
        let tokens = estimate_tool_schema_tokens(&agent, None, &reg);
        // The `act` agent allowlist exposes only `task` + `bash` to the model,
        // so the schema JSON (~1.4k chars) serialises to ~340 tokens. That is
        // a meaningful, non-negligible cost that must be counted (>200 guards
        // against an accidental empty/zero schema while tolerating the real
        // allowlist surface).
        assert!(
            tokens > 200,
            "act agent tool schemas should cost >200 tokens, got {tokens}"
        );
    }

    #[test]
    fn estimate_tool_schema_tokens_plan_excludes_build_hint() {
        // Plan mode rewrites the task tool description (no 'build' mention),
        // so the estimate may differ slightly — but both must be non-trivial.
        let act_agent = opencoder_core::resolve_agent("act").expect("act agent");
        let plan_agent = opencoder_core::resolve_agent("plan").expect("plan agent");
        let reg = registry();
        let act_tokens = estimate_tool_schema_tokens(&act_agent, None, &reg);
        let plan_tokens = estimate_tool_schema_tokens(&plan_agent, None, &reg);
        assert!(act_tokens > 200, "act tokens: {act_tokens}");
        assert!(plan_tokens > 200, "plan tokens: {plan_tokens}");
    }

    #[test]
    fn estimate_tool_schema_tokens_act_task_plan_matches_runner() {
        // The estimator must mirror the runner's schema build exactly. With
        // task-plan active, act and plan see the SAME schema set (question
        // visible, build hidden), so their estimates must coincide — if the
        // estimator forgot the task-plan build-strip, act would come out
        // strictly larger.
        let act_agent = opencoder_core::resolve_agent("act").expect("act agent");
        let plan_agent = opencoder_core::resolve_agent("plan").expect("plan agent");
        let reg = registry();
        let plan = Some("> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\nbody");
        let act_plan = estimate_tool_schema_tokens(&act_agent, plan, &reg);
        let plan_plan = estimate_tool_schema_tokens(&plan_agent, plan, &reg);
        assert_eq!(
            act_plan, plan_plan,
            "act+task-plan must estimate identically to plan"
        );
    }
}
