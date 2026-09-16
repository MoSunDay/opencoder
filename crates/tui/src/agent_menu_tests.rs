//! Tests for the `/agent` picker: catalog order, fuzzy semantics
//! (name-first, description fallback), key handling and the pick token.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn card(name: &str, desc: &str) -> AgentCard {
    AgentCard {
        name: name.into(),
        description: desc.into(),
    }
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

#[test]
fn empty_query_lists_every_agent_in_order() {
    let m = AgentMenu::new(vec![
        card("act", "d"),
        card("writer", "w"),
        card("plan", "p"),
    ]);
    assert_eq!(m.visible_count(), 3);
    assert_eq!(
        m.visible_agents()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["act", "writer", "plan"]
    );
}

#[test]
fn fuzzy_filter_matches_the_name_first_and_description_as_fallback() {
    let mut m = AgentMenu::new(vec![
        card("coder", "Custom agent coder"),
        card("writer", "small diffs"),
        card("plan", "read-only explorer"),
    ]);
    for c in "wr".chars() {
        m.on_char(c);
    }
    // 'wr' is a subsequence of 'writer' but of no other name/description
    // here -- the row set narrows to the name match.
    assert_eq!(m.visible_count(), 1);
    assert_eq!(m.selected_agent().unwrap().name, "writer");
    // Description fallback: a query that misses every name can still hit
    // a description ('read-only explorer' fuzzy-matches 'explr').
    let mut m = AgentMenu::new(vec![
        card("coder", "bash and subagents"),
        card("plan", "Read-only plan agent. Explores code."),
    ]);
    for c in "explr".chars() {
        m.on_char(c);
    }
    assert_eq!(m.visible_count(), 1);
    assert_eq!(m.selected_agent().unwrap().name, "plan");
}

#[test]
fn best_fuzzy_score_sorts_first_and_a_miss_empties_the_menu() {
    let mut m = AgentMenu::new(vec![
        card("c_o_d_r", "scattered"),
        card("coder", "compact prefix"),
        card("sandy", "no match"),
    ]);
    for c in "cod".chars() {
        m.on_char(c);
    }
    assert_eq!(
        m.visible_agents()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["coder", "c_o_d_r"],
        "compact prefix must rank ahead of the scattered subsequence"
    );
    for _ in 0..3 {
        m.on_backspace();
    }
    for c in "zzz".chars() {
        m.on_char(c);
    }
    assert_eq!(m.visible_count(), 0, "no subsequence, no rows");
    assert_eq!(m.selected_agent(), None);
}

#[test]
fn enter_and_tab_pick_the_highlighted_agent_and_close_the_menu() {
    for close_key in [KeyCode::Enter, KeyCode::Tab] {
        let mut slot = Some(AgentMenu::new(vec![card("writer", "w"), card("act", "a")]));
        m_down(&mut slot);
        let outcome = handle_agent_key(&mut slot, KeyEvent::new(close_key, KeyModifiers::NONE));
        assert_eq!(outcome, AgentOutcome::Pick("act".into()));
        assert!(slot.is_none(), "pick closes the menu");
    }
}

#[test]
fn esc_closes_without_picking_and_ctrl_d_quits() {
    let mut slot = Some(AgentMenu::new(vec![card("act", "a")]));
    assert_eq!(
        handle_agent_key(&mut slot, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        AgentOutcome::Idle
    );
    assert!(slot.is_none());
    let mut slot = Some(AgentMenu::new(vec![card("act", "a")]));
    assert_eq!(
        handle_agent_key(
            &mut slot,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        ),
        AgentOutcome::Quit
    );
    assert!(slot.is_none());
    // Closed slot: every keystroke is Idle (caller's modal mode owns it).
    let mut none = None;
    assert_eq!(handle_agent_key(&mut none, key('a')), AgentOutcome::Idle);
}

fn m_down(slot: &mut Option<AgentMenu>) {
    if let Some(m) = slot.as_mut() {
        m.move_down();
    }
}

#[test]
fn pick_token_carries_the_control_head_with_trailing_space() {
    assert_eq!(pick_token("writer"), "/agent writer ");
}

#[test]
fn available_cards_start_with_builtin_primaries_without_workflow() {
    let cards = available_primary_agents();
    let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        &names[..3.min(names.len())],
        &["act", "plan", "command"][..3.min(names.len())]
    );
    assert!(!names.contains(&"workflow"), "scheduler role is excluded");
    assert!(!names.contains(&"explore"), "subagents are not switchable");
    // Without an agents-root override there are no file cards, so builtin
    // descriptions come through verbatim.
    let act = cards.iter().find(|c| c.name == "act").unwrap();
    assert!(act.description.contains("Default execution agent"));
}

#[test]
fn file_agents_merge_in_with_soul_description_and_generic_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let (_lock, _guard) = override_agents(dir.path());
    // writer: a real prompt pool with a soul.md first line.
    write_file_agent(dir.path(), "writer", "Writer soul: small diffs.\nmore");
    // bare: a card with no prompt reference -> generic fallback.
    std::fs::create_dir_all(dir.path().join("bare")).unwrap();
    std::fs::write(
        dir.path().join("bare").join("meta.json"),
        r#"{ "name": "bare", "current": { "prompt": "missing" } }"#,
    )
    .unwrap();

    let cards = available_primary_agents();
    let writer = cards
        .iter()
        .find(|c| c.name == "writer")
        .expect("writer listed");
    assert_eq!(writer.description, "Writer soul: small diffs.");
    let bare = cards
        .iter()
        .find(|c| c.name == "bare")
        .expect("bare listed");
    assert_eq!(bare.description, "Custom agent bare");
}

// -- fixtures ----------------------------------------------------------

/// Override guard that RESETS the process-global root on drop, so the
/// next test in this binary never sees a deleted tempdir.
struct AgentRootGuard;

impl Drop for AgentRootGuard {
    fn drop(&mut self) {
        opencoder_core::agent::set_agents_dir_override(None);
    }
}

fn override_agents(root: &std::path::Path) -> (std::sync::MutexGuard<'static, ()>, AgentRootGuard) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    opencoder_core::agent::set_agents_dir_override(Some(root.to_path_buf()));
    (g, AgentRootGuard)
}

fn write_file_agent(root: &std::path::Path, name: &str, soul: &str) {
    let pool = root.join("prompts").join(name);
    let vdir = pool.join("v1");
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::write(vdir.join("soul.md"), soul).unwrap();
    std::fs::write(
        pool.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": 1, "history": [1] }}"#),
    )
    .unwrap();
    let adir = root.join(name);
    std::fs::create_dir_all(&adir).unwrap();
    std::fs::write(
        adir.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": {{ "prompt": "{name}" }} }}"#),
    )
    .unwrap();
}
