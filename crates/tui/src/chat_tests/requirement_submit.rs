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
// Arm lifecycle under consumption-time arming: entering plan starts a FRESH
// phase (any stale arm collapses); the ONLY path back to armed is the
// TurnDone(plan) re-arm from the persisted plan-phase counter (see
// app_loop_tests), which fires only after a real requirement ran in the
// phase. No submit-side flag exists anymore, so a stranded, never-consumed
// admit can never arm a context-clearing handoff.
// ---------------------------------------------------------------------------

#[test]
fn entering_plan_always_collapses_the_arm() {
    let mut v = ChatView {
        agent: "act".into(),
        plan_submitted: true, // stale from a previous phase
        ..Default::default()
    };
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert!(
        v.plan_submitted,
        "an act switch keeps the (sticky) flag; only plan entry collapses it"
    );
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        !v.plan_submitted,
        "entering plan must collapse a stale arm — fresh phase"
    );
    // ... and it stays unarmed until a TurnDone(plan) re-arm (out of scope
    // for this ChatView-level test — see app_loop_tests).
}
