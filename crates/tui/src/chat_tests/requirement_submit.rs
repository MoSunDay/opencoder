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
