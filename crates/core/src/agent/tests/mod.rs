use super::*;

/// Guards the `.replace()` in `base_prompt_plan()`: if BASE_PROMPT's
/// wording ever drifts so the replace becomes a no-op, the build subagent
/// advertisement silently leaks into the plan prompt. These assertions
/// fail loudly instead.
#[test]
fn plan_prompt_strips_build_subagent_advertisement() {
    // The exact clause targeted by `strip_build_delegation` (used by
    // `base_prompt_plan` and the session layer's task-plan stripping).
    // If this assertion fails, BASE_PROMPT has changed — update
    // `BUILD_DELEGATION_CLAUSE` to match the new wording.
    let replace_target = BUILD_DELEGATION_CLAUSE;
    assert!(
        base_prompt_act().contains(replace_target),
        "BASE_PROMPT no longer contains the '.replace()' target substring \
         {replace_target:?}. Update the .replace() call in base_prompt_plan()."
    );

    let plan = base_prompt_plan();

    // Safety property: the plan prompt must not advertise 'build'.
    assert!(
        !plan.contains("'build' (full tools)"),
        "plan prompt must not advertise the 'build' subagent, got: {plan}"
    );

    // Sanity: the 'explore' advertisement must survive (the replace should
    // only strip the build clause, not the entire delegation line).
    assert!(
        plan.contains("'explore' (read-only)"),
        "plan prompt must still advertise 'explore', got: {plan}"
    );
}

/// `question` is allowlisted for the two primary agents only (`plan` is
/// exempt from latent gating, `act` needs the task-plan skill unlock;
/// runtime visibility is gated elsewhere). Subagents never see it --
/// zero schema token cost. Structural guard (rules/01) against filter
/// drift.
#[test]
fn question_tool_is_plan_and_act_only() {
    for name in ["plan", "act"] {
        let a = resolve_agent(name).expect("primary agent registered");
        assert!(a.tools.allows("question"), "{name} must allow 'question'");
    }
    for other in ["explore", "build", "sidecar", "command", "workflow"] {
        let a = resolve_agent(other).expect("agent registered");
        assert!(
            !a.tools.allows("question"),
            "{other} must not allow 'question'"
        );
    }
}

/// Pin down the `sidecar` observer's tool set: read-only inspection only
/// (read/search/ls plus a classifier-gated bash), never mutating or
/// delegation tools. The sidecar answers questions about the main task
/// from a context snapshot; it must never be able to change state.
#[test]
fn sidecar_observer_is_read_only() {
    let sidecar = resolve_agent("sidecar").expect("sidecar agent registered");
    assert_eq!(sidecar.kind, AgentKind::Subagent);
    assert_eq!(sidecar.mode, AgentMode::Subagent);
    for allowed in &["read", "search", "ls", "bash"] {
        assert!(
            sidecar.tools.allows(allowed),
            "sidecar must allow '{allowed}'"
        );
    }
    for blocked in &["edit", "write", "task", "question"] {
        assert!(
            !sidecar.tools.allows(blocked),
            "sidecar (read-only) must not allow '{blocked}'"
        );
    }
    // The prompt states the observer contract: snapshot-in, progress-out,
    // read-only bash, and no modification claims.
    let prompt = sidecar.prompt;
    assert!(prompt.contains("sidecar observer"), "got: {prompt}");
    assert!(
        prompt.contains("read-only inspection commands"),
        "got: {prompt}"
    );
    assert!(prompt.contains("intercepted and refused"), "got: {prompt}");
    assert!(prompt.contains("CANNOT edit or write"), "got: {prompt}");
}

/// The plan prompt requires a focused plan without reviving the old rigid
/// Goal/TODO/Verify/Risks/Align template or an automatic act handoff.
#[test]
fn plan_prompt_is_read_only_without_question_advertisement() {
    let plan = base_prompt_plan();

    // Read-only constraints survive the rename.
    assert!(
        plan.contains("read-only"),
        "plan prompt must state its read-only constraints, got: {plan}"
    );
    assert!(
        plan.contains("Every state-changing tool attempt"),
        "plan prompt must note intercepted writes, got: {plan}"
    );
    assert!(plan.contains("output a plan only"), "got: {plan}");
    assert!(plan.contains("do not retry"), "got: {plan}");

    // No question-tool advertisement in the base prompt: the tool's
    // description lives ONLY in the task-plan skill body. (Generic prose
    // like "without asking questions" is fine — only the backticked
    // tool name or an explicit tool mention advertises the schema.)
    for banned in [
        "`question`",
        "prefer asking over assuming",
        "several in one turn",
        "looked up first, not asked",
    ] {
        assert!(
            !plan.contains(banned),
            "plan prompt must not advertise the question tool ({banned}), got: {plan}"
        );
    }

    // Plan-template semantics are gone.
    assert!(
        !plan.contains("Goal / TODO / Verify / Risks / Align"),
        "plan prompt must not require the plan template sections, got: {plan}"
    );
    assert!(
        !plan.contains("act mode"),
        "plan prompt must not hand off to a plan/act mode switch, got: {plan}"
    );
}

/// Pin down the `explore` subagent's exact tool set: it must carry
/// **only** `search` + `read` — the read-only pair. This is a structural
/// guard (rules/01): if the tool list drifts (e.g. an old `glob`/`grep`
/// creeps back, or a mutating tool leaks in) the test fails loudly.
#[test]
fn explore_subagent_carries_search_and_read_only() {
    let explore = resolve_agent("explore").expect("explore subagent registered");
    assert_eq!(explore.mode, AgentMode::Subagent);
    // Positive: the two read-only tools must be present.
    assert!(
        explore.tools.allows("search"),
        "explore must allow 'search'"
    );
    assert!(explore.tools.allows("read"), "explore must allow 'read'");
    // Negative: no mutating, delegation, or removed tools may leak in.
    for blocked in &["bash", "edit", "task", "write", "glob", "grep", "ls"] {
        assert!(
            !explore.tools.allows(blocked),
            "explore (read-only) must not allow '{blocked}'"
        );
    }
}

/// Pin down the `build` subagent's exact tool set: it must carry
/// **only** `bash` + `edit` — the implementation pair.
#[test]
fn build_subagent_carries_bash_and_edit_only() {
    let build = resolve_agent("build").expect("build subagent registered");
    assert_eq!(build.mode, AgentMode::Subagent);
    assert!(build.tools.allows("bash"), "build must allow 'bash'");
    assert!(build.tools.allows("edit"), "build must allow 'edit'");
    for blocked in &["search", "read", "task", "write", "glob", "grep", "ls"] {
        assert!(
            !build.tools.allows(blocked),
            "build (implementation) must not allow '{blocked}'"
        );
    }
}

/// The predicate every hide surface must derive from.
#[test]
fn build_delegation_hidden_matrix() {
    assert!(build_delegation_hidden(AgentKind::Plan, false));
    assert!(build_delegation_hidden(AgentKind::Plan, true));
    assert!(!build_delegation_hidden(AgentKind::Act, false));
    assert!(build_delegation_hidden(AgentKind::Act, true));
    assert!(!build_delegation_hidden(AgentKind::Subagent, false));
}

/// The `--prompt-file` preamble is a strip target exactly like the
/// BASE_PROMPT: it must contain the clause (else stripping is a no-op
/// and the test below proves nothing) and lose every 'build' mention
/// after stripping.
#[test]
fn tool_preamble_build_clause_is_strip_target() {
    assert!(tool_preamble().contains(BUILD_DELEGATION_CLAUSE));
    let stripped = strip_build_delegation(tool_preamble());
    assert!(!stripped.contains("'build'"));
    assert!(stripped.contains("## Tools"));
}

mod file_agents;
