//! Unit tests for the `/subtasks` fullscreen detail view: pure view logic
//! (timeline shaping, registry/snapshot merges, selection + key routing).

use super::*;
use crossterm::event::KeyModifiers;
use serde_json::json;

    fn entry(id: &str, kind: &str, status: &str) -> SubtaskRef {
        SubtaskRef {
            execution_id: id.into(),
            kind: kind.into(),
            capability_id: "ops_x".into(),
            question: format!("question {id}"),
            status: status.into(),
            result_summary: None,
            tokens: 0,
            created_at: 1,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn timeline_rows_maps_frame_shape_in_order() {
        let frames = [
            json!({"seq":1,"kind":"user_input","data":{},"ts":10_500}),
            json!({"seq":2,"kind":"llm_usage","data":{"total_tokens":7},"ts":11_900}),
        ];
        assert_eq!(
            timeline_rows(&frames, 10),
            vec!["1 user_input 10", "2 llm_usage 11"]
        );
        // Missing fields degrade to zeros instead of panicking.
        assert_eq!(timeline_rows(&[json!({})], 10), vec!["0 event 0"]);
        assert!(timeline_rows(&[], 10).is_empty());
    }

    #[test]
    fn timeline_rows_keeps_last_limit_with_marker() {
        let frames: Vec<Value> = (1..=250)
            .map(|seq| json!({"seq":seq,"kind":"text_delta","data":{},"ts":seq*1000}))
            .collect();
        let rows = timeline_rows(&frames, 200);
        assert_eq!(rows.len(), 201, "200 kept rows + the truncation marker");
        assert_eq!(rows[0], "\u{2026} 50 earlier events");
        assert_eq!(rows[1], "51 text_delta 51");
        assert_eq!(rows[200], "250 text_delta 250");
    }

    #[test]
    fn is_conversational_covers_agent_and_operator_only() {
        assert!(is_conversational("agent"));
        assert!(is_conversational("operator"));
        assert!(!is_conversational("todos"));
        assert!(!is_conversational("dag"));
        assert!(!is_conversational(""));
    }

    #[test]
    fn apply_update_merges_entry_and_snapshot() {
        let mut view = SubtaskView::new(vec![entry("agent-a", "agent", "running")]);
        view.snapshots.insert(
            "agent-a".into(),
            SubtaskSnapshot {
                execution_id: "agent-a".into(),
                status: "running".into(),
                summary: None,
                tokens: 3,
                timeline: vec!["1 user_input 1".into()],
            },
        );
        view.apply_update("agent-a", "done", Some("answer"), 90);
        let entry = &view.entries[0];
        assert_eq!(entry.status, "done");
        assert_eq!(entry.result_summary.as_deref(), Some("answer"));
        assert_eq!(entry.tokens, 90);
        let snapshot = &view.snapshots["agent-a"];
        assert_eq!(snapshot.status, "done");
        assert_eq!(snapshot.summary.as_deref(), Some("answer"));
        assert_eq!(snapshot.tokens, 90, "snapshot keeps its timeline");
        // Unknown ids are a no-op.
        view.apply_update("ghost", "done", None, 1);
        assert_eq!(view.entries[0].status, "done");
    }

    #[test]
    fn selection_clamps_and_resets_scroll() {
        let mut view = SubtaskView::new(vec![
            entry("a", "agent", "done"),
            entry("b", "agent", "done"),
            entry("c", "todos", "running"),
        ]);
        assert_eq!(view.selected, 2, "new view selects the newest entry");
        view.scroll = 5;
        view.move_selection(-1);
        assert_eq!(view.selected, 1);
        assert_eq!(view.scroll, 0, "selection change resets the scroll");
        view.move_selection(-5);
        assert_eq!(view.selected, 0, "clamped at the first row");
        view.move_selection(99);
        assert_eq!(view.selected, 2, "clamped at the last row");
        let mut empty = SubtaskView::new(Vec::new());
        empty.move_selection(3);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn esc_closes_and_enter_routes_by_kind() {
        let mut view = SubtaskView::new(vec![entry("agent-a", "agent", "done")]);
        assert_eq!(handle_key(&mut view, key(KeyCode::Esc), 24, &config()), SubtaskOutcome::Close);
        assert_eq!(
            handle_key(&mut view, key(KeyCode::Enter), 24, &config()),
            SubtaskOutcome::EnterConversation("agent-a".into())
        );
        let mut todos = SubtaskView::new(vec![entry("todo-a", "todos", "running")]);
        assert_eq!(
            handle_key(&mut todos, key(KeyCode::Enter), 24, &config()),
            SubtaskOutcome::Idle,
            "non-conversational kinds must not descend"
        );
    }

    #[test]
    fn ctrl_enter_appends_selected_and_only_with_entries() {
        let mut view = SubtaskView::new(vec![entry("agent-a", "agent", "done")]);
        let ctrl_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        assert_eq!(
            handle_key(&mut view, ctrl_enter, 24, &config()),
            SubtaskOutcome::Append("agent-a".into())
        );
        let mut empty = SubtaskView::new(Vec::new());
        assert_eq!(
            handle_key(&mut empty, ctrl_enter, 24, &config()),
            SubtaskOutcome::Idle,
            "no entry selected: nothing to append"
        );
    }

    #[test]
    fn page_scroll_is_bounded_by_last_detail_lines() {
        let mut view = SubtaskView::new(vec![entry("agent-a", "agent", "done")]);
        view.last_detail_lines.set(10);
        view.scroll_down(100);
        assert_eq!(view.scroll, 10, "bounded at the rendered height");
        view.scroll_up(100);
        assert_eq!(view.scroll, 0);
    }

