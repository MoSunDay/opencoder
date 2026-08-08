use super::super::*;

fn make_subagent(started_at_ms: i64, elapsed_ms: Option<u64>, done: bool) -> ChatBlock {
    ChatBlock::Subagent {
        id: "s1".into(),
        child_session_id: "c1".into(),
        kind: "explore".into(),
        prompt: "find foo".into(),
        view: ChatView::default(),
        done,
        ok: done,
        cancelled: false,
        summary: if done {
            "found it".into()
        } else {
            String::new()
        },
        started_at_ms,
        elapsed_ms,
    }
}

fn flat_text(v: &ChatView, now_ms: i64) -> String {
    v.flatten_with(0, now_ms)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

// --- Subagent timer tests ---

#[test]
fn running_subagent_shows_live_timer() {
    let mut v = ChatView::default();
    v.blocks.push(make_subagent(1000, None, false));
    let text = flat_text(&v, 6000);
    assert!(
        text.contains("5s"),
        "running subagent should show live timer; got: {text}"
    );
}

#[test]
fn done_subagent_freezes_duration() {
    let mut v = ChatView::default();
    v.blocks.push(make_subagent(1000, Some(18000), true));
    let text = flat_text(&v, 100000);
    assert!(
        text.contains("18s"),
        "done subagent should show frozen 18s; got: {text}"
    );
}

#[test]
fn done_subagent_hides_subsecond() {
    let mut v = ChatView::default();
    v.blocks.push(make_subagent(1000, Some(500), true));
    let text = flat_text(&v, 100000);
    assert!(
        !text.contains("0s"),
        "sub-second done subagent duration should be hidden; got: {text}"
    );
}
