//! Inline tests extracted from `lib.rs` to respect the 800-line file cap.
//! Compiled as `#[cfg(test)] mod lib_tests` via `#[path]`; the chained
//! `use super::*` glob keeps every test resolving against the crate root
//! exactly as it did when the modules were declared inline.

use super::*;

#[cfg(test)]
mod cache_salt_tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session(cache_salt: Option<bool>) -> SessionState {
        // `cache_salt_for` never touches the filesystem, so a plain temp path
        // (kept alive for the test's duration by the caller) suffices. We use
        // a stable subdir under the OS temp dir rather than a TempDir so the
        // SessionState owns a valid PathBuf without juggling drop lifetimes.
        let working_dir = std::env::temp_dir().join("opencoder-cache-salt-tests");
        SessionState::new(
            "sess-123",
            resolve_agent("act").unwrap(),
            Config {
                cache_salt,
                ..Config::default()
            },
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            working_dir,
        )
    }

    #[test]
    fn derives_salt_when_enabled() {
        let s = make_session(Some(true));
        assert_eq!(cache_salt_for(&s).as_deref(), Some("act:sess-123"));
    }

    #[test]
    fn no_salt_when_disabled_or_unset() {
        let s = make_session(Some(false));
        assert_eq!(cache_salt_for(&s), None);
        let s = make_session(None);
        assert_eq!(cache_salt_for(&s), None);
    }

    /// Build a fresh SharedCancel (turn-level token wrapper) for tests.
    fn new_shared_cancel() -> SharedCancel {
        Arc::new(std::sync::Mutex::new(CancellationToken::new()))
    }

    /// Check whether a SharedCancel token has been fired.
    fn is_shared_cancelled(tc: &SharedCancel) -> bool {
        tc.lock().map(|g| g.is_cancelled()).unwrap_or(false)
    }

    #[test]
    fn fire_child_cancels_returns_false_on_empty_registry() {
        let registry: Arc<Mutex<HashMap<String, CancellationToken>>> =
            Arc::new(Mutex::new(HashMap::new()));
        assert!(!fire_child_cancels(&registry));
    }

    #[test]
    fn fire_child_cancels_cancels_all_registered_tokens() {
        let t1 = CancellationToken::new();
        let t2 = CancellationToken::new();
        let mut map = HashMap::new();
        map.insert("child-1".to_string(), t1.clone());
        map.insert("child-2".to_string(), t2.clone());
        let registry = Arc::new(Mutex::new(map));

        assert!(fire_child_cancels(&registry));
        assert!(t1.is_cancelled());
        assert!(t2.is_cancelled());
    }

    #[test]
    fn fire_child_turn_cancel_returns_false_on_empty_registry() {
        let registry: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        assert!(!fire_child_turn_cancel(&registry, "child-x"));
    }

    #[test]
    fn fire_child_turn_cancel_returns_false_for_unknown_call_id() {
        let t1 = new_shared_cancel();
        let mut map = HashMap::new();
        map.insert("child-1".to_string(), t1);
        let registry = Arc::new(Mutex::new(map));
        assert!(!fire_child_turn_cancel(&registry, "child-2"));
    }

    #[test]
    fn fire_child_turn_cancel_fires_only_targeted_token() {
        let t1 = new_shared_cancel();
        let t2 = new_shared_cancel();
        let mut map = HashMap::new();
        map.insert("child-1".to_string(), t1.clone());
        map.insert("child-2".to_string(), t2.clone());
        let registry = Arc::new(Mutex::new(map));

        assert!(fire_child_turn_cancel(&registry, "child-1"));
        assert!(is_shared_cancelled(&t1), "targeted token must be cancelled");
        assert!(
            !is_shared_cancelled(&t2),
            "non-targeted token must stay uncancelled"
        );
    }

    #[test]
    fn fire_turn_cancel_fires_supplied_token() {
        let token: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
        assert!(!token.lock().unwrap().is_cancelled());
        fire_turn_cancel(&token);
        assert!(token.lock().unwrap().is_cancelled());
    }
}

#[cfg(test)]
mod plan_tag_tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_plan_session() -> SessionState {
        let config = Config::default();
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "test",
            resolve_agent("plan").unwrap(),
            config,
            client,
            PathBuf::from("."),
        )
    }

    fn make_act_session() -> SessionState {
        let config = Config::default();
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            config,
            client,
            PathBuf::from("."),
        )
    }

    #[test]
    fn plan_first_prompt_not_tagged() {
        let mut s = make_plan_session();
        let mut text = String::from("build a web app");
        s.maybe_tag_plan_prompt(&mut text);
        assert_eq!(text, "build a web app", "first prompt should not be tagged");
        assert_eq!(s.plan_input_count, 1);
    }

    #[test]
    fn plan_second_prompt_tagged() {
        let mut s = make_plan_session();
        s.plan_input_count = 1;
        let mut text = String::from("also add tests");
        s.maybe_tag_plan_prompt(&mut text);
        assert!(text.contains("（当前处于只读的 plan 模式，聚焦计划生成）"));
        assert_eq!(s.plan_input_count, 2);
    }

    #[test]
    fn act_mode_never_tagged() {
        let mut s = make_act_session();
        s.plan_input_count = 5; // even with prior count, act mode should not tag
        let mut text = String::from("do something");
        s.maybe_tag_plan_prompt(&mut text);
        assert_eq!(text, "do something", "act mode should never tag");
    }

    #[test]
    fn switch_to_plan_resets_count() {
        let mut s = make_plan_session();
        s.plan_input_count = 3;
        // simulate ClearContext handoff reset
        s.after_handoff(0, String::new());
        assert_eq!(s.plan_input_count, 0, "after_handoff resets count");
    }
}

#[cfg(test)]
mod compaction_after_handoff_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session() -> SessionState {
        let config = Config::default();
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            config,
            client,
            PathBuf::from("."),
        )
    }

    /// After a plan→act handoff, compaction must clear the stale handoff
    /// boundary and install a compaction summary instead — otherwise resume
    /// would take the handoff path (it checks `handoff_seq` first) and ignore
    /// the freshly written summary.
    #[test]
    fn compaction_after_handoff_clears_handoff_state() {
        let mut s = make_session();
        // Simulate post-handoff state: handoff_seq set, no compaction yet.
        s.after_handoff(10, "the plan".into());
        assert_eq!(s.handoff_seq, Some(10));
        assert!(s.summary_seq.is_none());

        // prev_skip must fall back to handoff_seq when summary_seq is None.
        let prev_skip = s.summary_seq.or(s.handoff_seq).unwrap_or(0);
        assert_eq!(prev_skip, 10, "prev_skip must use handoff_seq");

        // Simulate compaction producing a summary covering the handoff head.
        s.after_compaction("compacted summary".into(), prev_skip);

        assert_eq!(
            s.summary_seq,
            Some(10),
            "summary_seq should hold the (handoff-derived) skip"
        );
        assert!(s.handoff_seq.is_none(), "handoff_seq must be cleared");
        assert!(s.handoff_plan.is_none(), "handoff_plan must be cleared");
        assert_eq!(s.summary.as_deref(), Some("compacted summary"));
    }

    /// With no prior handoff and no prior compaction, prev_skip is 0.
    #[test]
    fn prev_skip_zero_when_no_compaction_or_handoff() {
        let s = make_session();
        let prev_skip = s.summary_seq.or(s.handoff_seq).unwrap_or(0);
        assert_eq!(prev_skip, 0);
    }

    /// When a compaction summary already exists it takes priority over a
    /// (hypothetical leftover) handoff_seq.
    #[test]
    fn summary_seq_takes_priority_over_handoff_seq() {
        let mut s = make_session();
        s.handoff_seq = Some(5);
        s.summary_seq = Some(20);
        let prev_skip = s.summary_seq.or(s.handoff_seq).unwrap_or(0);
        assert_eq!(prev_skip, 20);
        s.after_compaction("s".into(), 20);
        assert!(s.handoff_seq.is_none());
    }
}

#[cfg(test)]
mod store_message_count_tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session() -> SessionState {
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            PathBuf::from("."),
        )
    }

    #[test]
    fn store_message_count_no_synthetic_head() {
        let mut s = make_session();
        s.messages.push(Message::user("u1", "hi"));
        s.messages.push(Message::assistant("a1"));
        assert_eq!(s.store_message_count(), 2);
    }

    #[test]
    fn store_message_count_with_summary_seq() {
        let mut s = make_session();
        s.summary_seq = Some(5);
        // The synthetic summary at index 0 is NOT in the store, so the
        // store count is 5 + (2 - 1) = 6.
        s.messages.push(Message::user("u1", "summary"));
        s.messages.push(Message::assistant("a1"));
        assert_eq!(s.store_message_count(), 6);
    }

    #[test]
    fn store_message_count_with_handoff_seq() {
        let mut s = make_session();
        s.handoff_seq = Some(3);
        s.messages.push(Message::user("u1", "handoff"));
        s.messages.push(Message::assistant("a1"));
        // 3 + (2 - 1) = 4
        assert_eq!(s.store_message_count(), 4);
    }

    #[test]
    fn store_message_count_empty_with_summary_seq_does_not_overflow() {
        // This is the bug: messages.len() == 0 with summary_seq set would
        // underflow. saturating_sub prevents the panic.
        let mut s = make_session();
        s.summary_seq = Some(5);
        s.messages.clear();
        // skip=5 + saturating_sub(0,1)=0 = 5
        assert_eq!(s.store_message_count(), 5);
    }

    #[test]
    fn store_message_count_empty_with_handoff_seq_does_not_overflow() {
        let mut s = make_session();
        s.handoff_seq = Some(3);
        s.messages.clear();
        assert_eq!(s.store_message_count(), 3);
    }
}

#[cfg(test)]
mod effective_ap_mode_tests {
    //! `SessionState::effective_ap_mode` precedence: `None` follows the
    //! config mode; any session-scoped override (`/ap` session-only or
    //! restored on resume) wins regardless of the config value.

    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, ApMode, AutoPilotConfig, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session(config_mode: ApMode) -> SessionState {
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            Config {
                autopilot: AutoPilotConfig {
                    mode: config_mode,
                    ..AutoPilotConfig::default()
                },
                ..Config::default()
            },
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            PathBuf::from("."),
        )
    }

    #[test]
    fn none_follows_config_mode() {
        let s = make_session(ApMode::Review);
        assert_eq!(s.ap_mode_override, None, "fresh sessions start clean");
        assert_eq!(s.effective_ap_mode(), ApMode::Review);
    }

    #[test]
    fn override_wins_over_config() {
        let mut s = make_session(ApMode::Ap);
        s.ap_mode_override = Some(ApMode::Off);
        assert_eq!(
            s.effective_ap_mode(),
            ApMode::Off,
            "Some(Off) beats config Ap"
        );

        s.ap_mode_override = Some(ApMode::Ap);
        assert_eq!(s.effective_ap_mode(), ApMode::Ap, "Some(Ap) returned");

        s.ap_mode_override = Some(ApMode::Review);
        assert_eq!(
            s.effective_ap_mode(),
            ApMode::Review,
            "Some(Review) returned"
        );
    }
}
