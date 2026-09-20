//! Tests for the `/agent` picker: row order, fuzzy semantics
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
        card("coder", "d"),
        card("writer", "w"),
        card("reviewer", "p"),
    ]);
    assert_eq!(m.visible_count(), 3);
    assert_eq!(
        m.visible_agents()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["coder", "writer", "reviewer"]
    );
}

#[test]
fn fuzzy_filter_matches_the_name_first_and_description_as_fallback() {
    let mut m = AgentMenu::new(vec![
        card("coder", "Custom agent coder"),
        card("writer", "small diffs"),
        card("inspector", "read-only explorer"),
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
        card("inspector", "Read-only custom agent. Explores code."),
    ]);
    for c in "explr".chars() {
        m.on_char(c);
    }
    assert_eq!(m.visible_count(), 1);
    assert_eq!(m.selected_agent().unwrap().name, "inspector");
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
        let mut slot = Some(AgentMenu::new(vec![
            card("writer", "w"),
            card("coder", "a"),
        ]));
        m_down(&mut slot);
        let outcome = handle_agent_key(&mut slot, KeyEvent::new(close_key, KeyModifiers::NONE));
        assert_eq!(outcome, AgentOutcome::Pick("coder".into()));
        assert!(slot.is_none(), "pick closes the menu");
    }
}

#[test]
fn esc_closes_without_picking_and_ctrl_d_quits() {
    let mut slot = Some(AgentMenu::new(vec![card("coder", "a")]));
    assert_eq!(
        handle_agent_key(&mut slot, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        AgentOutcome::Idle
    );
    assert!(slot.is_none());
    let mut slot = Some(AgentMenu::new(vec![card("coder", "a")]));
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
fn enter_and_tab_close_empty_results_without_picking() {
    for cards in [vec![], vec![card("writer", "w")]] {
        for close_key in [KeyCode::Enter, KeyCode::Tab] {
            let mut menu = AgentMenu::new(cards.clone());
            menu.on_char('z');
            menu.move_up();
            menu.move_down();
            assert_eq!(menu.visible_count(), 0);
            assert_eq!(menu.selected_agent(), None);
            let mut slot = Some(menu);
            assert_eq!(
                handle_agent_key(&mut slot, KeyEvent::new(close_key, KeyModifiers::NONE)),
                AgentOutcome::Idle
            );
            assert!(slot.is_none());
        }
    }
}

#[test]
fn popup_distinguishes_no_custom_agents_from_no_search_matches() {
    for (cards, expected, absent) in [
        (vec![], "no custom agents available", "no matching agent"),
        (
            vec![card("writer", "w")],
            "no matching agent",
            "no custom agents available",
        ),
    ] {
        let mut menu = AgentMenu::new(cards);
        menu.on_char('z');
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_agent_popup(frame, area, 20, &menu);
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains(absent), "{text}");
    }
}
