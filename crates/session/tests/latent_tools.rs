//! Integration tests: latent tools (`question`, `ssh_pty`) are hidden from the
//! model by default and only appear when their owning skill is activated —
//! with no agent exemptions: the sandbox agent sees `question` only while a
//! task-plan body is active, like every other agent.

use std::collections::HashSet;

use opencoder_core::{resolve_agent, ToolFilter};
use opencoder_session::tools::{latent, registry, schema_for};

/// Build the set of tool names that would be sent to the model, given an
/// agent tool filter and an optional skill body. Uses the same shared
/// visibility predicate as the runner's tool filter.
fn visible_tool_names(agent_filter: &ToolFilter, skill_body: Option<&str>) -> Vec<String> {
    let reg = registry();
    let unlocked = latent::unlocked_from_body(skill_body);
    let agent = opencoder_core::Agent {
        name: "probe".into(),
        kind: opencoder_core::AgentKind::Act,
        mode: opencoder_core::AgentMode::Primary,
        description: String::new(),
        prompt: String::new(),
        tools: agent_filter.clone(),
    };
    let mut allowed: Vec<String> = reg
        .keys()
        .filter(|name| latent::is_visible(name, &agent, &unlocked))
        .cloned()
        .collect();
    allowed.sort();
    allowed
}

#[test]
fn latent_tools_hidden_by_default() {
    // Use ToolFilter::All so the agent filter doesn't hide them — the only
    // thing hiding them should be the latent filter.
    let filter = ToolFilter::All;

    let names = visible_tool_names(&filter, None);

    // ssh_pty AND question must NOT appear for a plain act-shaped agent.
    assert!(
        !names.contains(&"ssh_pty".to_string()),
        "ssh_pty should be hidden by default, got: {names:?}"
    );
    assert!(
        !names.contains(&"question".to_string()),
        "question should be hidden without the task-plan skill, got: {names:?}"
    );

    // But normal tools like bash/read should appear.
    assert!(names.contains(&"bash".to_string()));
    assert!(names.contains(&"read".to_string()));
}

#[test]
fn latent_tools_unlocked_by_skill_body() {
    let filter = ToolFilter::All;

    // Simulate the ssh-pty skill body being active.
    let body = "# ssh-pty skill\n\nUse ssh_pty for persistent SSH.";
    let names = visible_tool_names(&filter, Some(body));
    assert!(
        names.contains(&"ssh_pty".to_string()),
        "ssh_pty should be unlocked by its skill body, got: {names:?}"
    );
}

#[test]
fn question_unlocked_by_plan_skill_body_only() {
    let filter = ToolFilter::All;

    let plan = "# task-plan\n\n## Overview\n\nPlan in phases; ask via question.";
    let names = visible_tool_names(&filter, Some(plan));
    assert!(
        names.contains(&"question".to_string()),
        "task-plan body must unlock question, got: {names:?}"
    );
}

#[test]
fn question_not_unlocked_by_review_skill_body() {
    // review no longer owns the question tool: naming itself in the prefix
    // window must not surface it, even if the body still mentions the tool.
    let filter = ToolFilter::All;
    let review = "# review\n\nEvidence-driven assessment; use question when blocked.";
    let names = visible_tool_names(&filter, Some(review));
    assert!(
        !names.contains(&"question".to_string()),
        "review body must NOT unlock question, got: {names:?}"
    );
}

#[test]
fn question_not_unlocked_by_mentioning_the_tool_name() {
    // Matching the tool name alone is deliberately insufficient — only the
    // task-plan skill name (inside the 500-char prefix) unlocks.
    let names = visible_tool_names(&ToolFilter::All, Some("ask via question when blocked"));
    assert!(
        !names.contains(&"question".to_string()),
        "a passing mention of 'question' must not unlock it, got: {names:?}"
    );
}

fn sandbox_visible_tools(skill_body: Option<&str>) -> Vec<String> {
    let sandbox = resolve_agent("sandbox").unwrap();
    let reg = registry();
    let unlocked: HashSet<&str> = latent::unlocked_from_body(skill_body);
    let mut names: Vec<String> = reg
        .keys()
        .filter(|n| latent::is_visible(n, &sandbox, &unlocked))
        .cloned()
        .collect();
    names.sort();
    names
}

#[test]
fn sandbox_hides_question_without_any_skill() {
    // No agent exemptions: sandbox hides question until the task-plan
    // skill unlocks it.
    let names = sandbox_visible_tools(None);
    assert!(
        !names.contains(&"question".to_string()),
        "sandbox must NOT see question with no skill, got: {names:?}"
    );
    // ssh_pty stays latent for sandbox with or without a skill.
    assert!(!names.contains(&"ssh_pty".to_string()));

    let unlocked = sandbox_visible_tools(Some("# task-plan\n\nPlan; ask via question."));
    assert!(
        unlocked.contains(&"question".to_string()),
        "an active task-plan body must unlock question for sandbox, got: {unlocked:?}"
    );
    assert!(!unlocked.contains(&"ssh_pty".to_string()));
}

#[test]
fn latent_tools_appear_in_schema_when_unlocked() {
    let reg = registry();
    let probe = opencoder_core::Agent {
        name: "probe".into(),
        kind: opencoder_core::AgentKind::Act,
        mode: opencoder_core::AgentMode::Primary,
        description: String::new(),
        prompt: String::new(),
        tools: ToolFilter::All,
    };

    // Without skill: schemas should not include ssh_pty.
    let unlocked = latent::unlocked_from_body(None);
    let allowed: std::collections::HashMap<_, _> = reg
        .iter()
        .filter(|(name, _)| latent::is_visible(name, &probe, &unlocked))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let schemas = schema_for(&allowed, false);
    let schema_names: Vec<&str> = schemas
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap())
        .collect();
    assert!(!schema_names.contains(&"ssh_pty"));

    // With skill: ssh_pty should appear in schemas.
    let unlocked2 = latent::unlocked_from_body(Some("ssh-pty ssh_pty"));
    let allowed2: std::collections::HashMap<_, _> = reg
        .iter()
        .filter(|(name, _)| latent::is_visible(name, &probe, &unlocked2))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let schemas2 = schema_for(&allowed2, false);
    let schema_names2: Vec<&str> = schemas2
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap())
        .collect();
    assert!(schema_names2.contains(&"ssh_pty"));
}
