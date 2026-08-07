use super::super::*;

// ---------------------------------------------------------------------------
// plan_submitted arming: every requirement delivery path (Enter-submit and
// Tab-queue while running) must count as a requirement submission in plan
// mode, otherwise Shift+Tab plan→act would do a pure switch and keep the full
// planning transcript instead of handing off with a cleared context.
// ---------------------------------------------------------------------------

#[test]
fn requirement_submitted_in_plan_arms_handoff() {
    let mut v = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };
    v.note_requirement_submitted();
    assert!(
        v.plan_submitted,
        "a queued/submitted requirement in plan mode must arm the plan→act handoff"
    );
}

#[test]
fn requirement_submitted_in_act_does_not_arm_handoff() {
    let mut v = ChatView {
        agent: "act".into(),
        plan_submitted: false,
        ..Default::default()
    };
    v.note_requirement_submitted();
    assert!(
        !v.plan_submitted,
        "requirement deliveries in act mode must never arm the plan→act handoff"
    );
}

#[test]
fn requirement_submitted_in_plan_then_act_switch_keeps_flag() {
    // app.rs reads the flag BEFORE the AgentSwitch event arrives, so arming
    // in plan mode must survive until the handoff decision is made.
    let mut v = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };
    v.note_requirement_submitted();
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert!(
        v.plan_submitted,
        "arming must survive the plan→act switch (app reads it first)"
    );
}

#[test]
fn requirement_submitted_then_reenter_plan_resets_flag() {
    // A fresh plan session starts unarmed: the handoff only fires when the
    // user actually submitted a requirement during THIS plan session.
    let mut v = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };
    v.note_requirement_submitted();
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        !v.plan_submitted,
        "re-entering plan mode must reset the arming"
    );
}

// ---------------------------------------------------------------------------
// Deferred arming for compound `/plan <content>` submitted from act mode:
// the submit path sets `pending_plan_arm`; the async `AgentSwitch("plan")`
// consumes it to re-arm `plan_submitted` (which that event would otherwise
// reset). Shift+Tab after the plan turn then keeps the plan and starts the
// task instead of plain-swapping.
// ---------------------------------------------------------------------------

#[test]
fn compound_plan_from_act_rearms_handoff_on_plan_switch() {
    let mut v = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    // What the Submit/Steer/Queue paths do for `/plan <content>`.
    v.pending_plan_arm = true;
    // Runner applies the mode switch asynchronously.
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        v.plan_submitted,
        "compound /plan from act must arm the plan->act handoff"
    );
    assert!(
        !v.pending_plan_arm,
        "deferred arming must be consumed by the switch event"
    );
}

#[test]
fn compound_plan_rearm_survives_transcript_reset() {
    // Compaction mid-plan-turn must not drop the deferred arming, mirroring
    // the plan_submitted preservation in fold_ui_events.
    let mut v = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    v.pending_plan_arm = true;
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    v.apply(&SessionEvent::TranscriptReset(Vec::new()));
    assert!(
        v.plan_submitted,
        "transcript reset must preserve the armed handoff"
    );
}

#[test]
fn stale_pending_arm_consumed_on_non_plan_switch() {
    // A pending flag must never survive a switch to a non-plan agent: a later
    // bare `/plan` re-entry would otherwise re-arm without a requirement.
    let mut v = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    v.pending_plan_arm = true;
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert!(!v.plan_submitted, "act switch must not arm the handoff");
    assert!(
        !v.pending_plan_arm,
        "stale pending arming must be consumed by any switch"
    );
    // Re-entering plan afterwards stays unarmed.
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        !v.plan_submitted,
        "re-entering plan after a dropped arm must stay unarmed"
    );
}
