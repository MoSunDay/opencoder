//! Turn-boundary ownership of the step ladder: ONE user input — submit,
//! steer consumption, or queue consumption — owns exactly ONE `N Steps +
//! Say` pairing. The rounds a later user input triggers must never merge
//! into an earlier turn's group (the "all turns' steps lumped together"
//! regression), and a ladder must never render above its own prompt echo.

use super::*;

fn tool(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: format!("{id}-out"),
        is_error: false,
        images: Vec::new(),
    });
}

fn groups(v: &ChatView) -> Vec<(usize, Vec<Vec<String>>)> {
    v.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ChatBlock::StepGroup { steps, .. } => Some((
                i,
                steps
                    .iter()
                    .map(|s| s.calls.iter().map(|c| c.id.clone()).collect())
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

fn user_indices(v: &ChatView) -> Vec<usize> {
    v.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ChatBlock::User { .. } => Some(i),
            _ => None,
        })
        .collect()
}

/// A steer absorbed mid-run is a NEW user input: the rounds it triggers own
/// a fresh ladder BELOW the echo, and the pre-steer ladder stays complete
/// with its own steps. Previously the post-steer rounds merged into the
/// pre-steer group — every turn's steps ended up in one ladder.
#[test]
fn steer_consumed_starts_a_new_ladder_below_the_echo() {
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("first prompt"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("T1R1 thinking".into()));
    tool(&mut v, "c1");

    v.apply(&SessionEvent::SteerConsumed {
        seq: 1,
        text: "and also check y".into(),
    });
    v.apply(&SessionEvent::ReasoningDelta("T1R2 thinking".into()));
    tool(&mut v, "c2");
    v.apply(&SessionEvent::TextDelta("final say".into()));
    v.apply(&SessionEvent::Done);

    let got = groups(&v);
    assert_eq!(got.len(), 2, "steer must split the ladder: {got:?}");
    assert_eq!(got[0].1, [["c1"]]);
    assert_eq!(got[1].1, [["c2"]]);

    // Settled order per turn contract: [User, Steps, (User, Steps)+ Say].
    let users = user_indices(&v);
    assert_eq!(users.len(), 2, "prompt + steer echo");
    assert!(
        got[0].0 > users[0],
        "pre-steer ladder sits below the prompt echo"
    );
    assert!(
        got[1].0 > users[1],
        "post-steer ladder sits below the steer echo"
    );
    let say = v
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Assistant { raw, .. } if raw == "final say"))
        .expect("say block");
    assert!(say > got[1].0, "say trails the steered turn's ladder");
}

/// The steer boundary also freezes the pre-steer ladder's animation: that
/// turn ended without its own say and can never animate again.
#[test]
fn steer_boundary_freezes_the_previous_ladder_progress() {
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("first prompt"),
    });
    v.begin_turn();
    tool(&mut v, "c1");
    // ToolStart armed the animation.
    assert!(matches!(
        &v.blocks[1],
        ChatBlock::StepGroup {
            progress_active: true,
            ..
        }
    ));
    v.apply(&SessionEvent::SteerConsumed {
        seq: 1,
        text: "redirect".into(),
    });
    assert!(
        matches!(
            &v.blocks[1],
            ChatBlock::StepGroup {
                progress_active: false,
                ..
            }
        ),
        "pre-steer ladder must stop animating at the boundary"
    );
}

/// A bare control command absorbed at the steer boundary echoes nothing and
/// must NOT split the ladder — it is not user content.
#[test]
fn bare_control_steer_keeps_one_ladder() {
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("first prompt"),
    });
    v.begin_turn();
    tool(&mut v, "c1");
    v.apply(&SessionEvent::SteerConsumed {
        seq: 1,
        text: String::new(),
    });
    tool(&mut v, "c2");
    let got = groups(&v);
    assert_eq!(got.len(), 1, "no echo, no split: {got:?}");
    assert_eq!(got[0].1, [["c1", "c2"]]);
}

/// Queue consumption at an idle boundary: `begin_turn` runs at the drain
/// restart, the echo lands afterwards, so the app re-anchors the floor below
/// the echo. The turn's ladder must render BELOW its own prompt echo —
/// previously the group was inserted at the stale floor above the echo.
#[test]
fn queue_consumed_ladder_renders_below_its_prompt_echo() {
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("first prompt"),
    });
    v.begin_turn();
    tool(&mut v, "c1");
    v.apply(&SessionEvent::TextDelta("first say".into()));
    v.apply(&SessionEvent::Done);

    // App layer: drain restart (begin_turn), then the QueueConsumed echo
    // push + floor re-anchor (app_loop.rs).
    v.begin_turn();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("queued prompt"),
    });
    v.push_marker(Line::from(""));
    v.reanchor_turn_after_user_echo();
    v.apply(&SessionEvent::ReasoningDelta("T2R1 thinking".into()));
    tool(&mut v, "c2");
    v.apply(&SessionEvent::TextDelta("second say".into()));
    v.apply(&SessionEvent::Done);

    let got = groups(&v);
    assert_eq!(got.len(), 2, "queued turn owns its own ladder: {got:?}");
    let users = user_indices(&v);
    assert_eq!(users.len(), 2);
    assert!(
        got[1].0 > users[1],
        "queued ladder must sit below the queued prompt echo"
    );
}

/// Multi-turn rendering sanity: sequential submits each render the
/// `[User, N Steps, Say]` triple — the flattened shape the turn contract
/// promises as the DEFAULT (collapsed) view.
#[test]
fn consecutive_turns_render_steps_then_say_each() {
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("first prompt"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("T1R1 thinking".into()));
    tool(&mut v, "c1");
    tool(&mut v, "c2");
    v.apply(&SessionEvent::ReasoningDelta("T1R2 thinking".into()));
    tool(&mut v, "c3");
    v.apply(&SessionEvent::TextDelta("first say".into()));
    v.apply(&SessionEvent::Done);

    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("second prompt"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::ReasoningDelta("T2R1 thinking".into()));
    tool(&mut v, "c4");
    v.apply(&SessionEvent::TextDelta("second say".into()));
    v.apply(&SessionEvent::Done);

    let text: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    let joined = text.join("\n");
    assert!(
        joined.contains("Say(2 steps): first say"),
        "turn one renders its merged collapsed header: {joined:?}"
    );
    assert!(
        joined.contains("Say(1 step): second say"),
        "turn two renders its own merged header: {joined:?}"
    );
    // Order contract: each turn's merged header precedes that turn's Say
    // body; the second prompt sits between the two pairs.
    let steps1 = joined.find("Say(2 steps)").unwrap();
    let say1 = joined.find("first say").unwrap();
    let prompt2 = joined.find("second prompt").unwrap();
    let steps2 = joined.find("Say(1 step)").unwrap();
    let say2 = joined.find("second say").unwrap();
    assert!(steps1 < say1 && say1 < prompt2 && prompt2 < steps2 && steps2 < say2);
}

// ----- pending_turn_echo: the echo memory that survives TranscriptReset -----

/// A consumed steer's echo is remembered: a subsequent TranscriptReset
/// rebuild (steered `/act_clear_context <tail>`) re-pushes it as the running
/// turn's user boundary instead of orphaning the ladder.
#[test]
fn steer_consumed_echo_is_remembered_for_reset_restore() {
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("first"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::SteerConsumed {
        seq: 1,
        text: "/act_clear_context 实现贪吃蛇".into(),
    });
    assert_eq!(v.pending_turn_echo.as_deref(), Some("实现贪吃蛇"));
    // Done retires the memory: a later bare reset must not resurrect it.
    v.apply(&SessionEvent::Done);
    assert!(v.pending_turn_echo.is_none());
}

/// A bare control command (empty echo, applied inline) never touches the
/// running turn's remembered echo.
#[test]
fn bare_control_consumed_leaves_pending_echo_untouched() {
    let mut v = ChatView {
        pending_turn_echo: Some("original prompt".into()),
        ..Default::default()
    };
    v.apply(&SessionEvent::SteerConsumed {
        seq: 7,
        text: String::new(),
    });
    assert_eq!(v.pending_turn_echo.as_deref(), Some("original prompt"));
}
