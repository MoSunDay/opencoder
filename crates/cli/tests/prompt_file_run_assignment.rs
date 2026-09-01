//! End-to-end coverage of the `--agent plan --prompt-file` assignment seam
//! (rules/01 regression test). The composition-only unit tests in
//! `run.rs` (`prompt_file_*_composition_*`) prove the composed text; the tests
//! here drive `apply_prompt_file` — the exact read→compose→assign function
//! `run_headless` (hence `main`) calls for `--prompt-file` — against a real
//! `SessionState`, so the `session.agent.prompt` store step itself is
//! regression-guarded instead of only diff-reviewed by hand.
//!
//! Pinned here:
//!   1. a plan agent stores the user body plus the tool preamble with the
//!      'build' delegation advertisement stripped (kind stays plan);
//!   2. an act agent stores the full preamble, 'build' clause included.

mod run {
    use std::path::Path;
    use std::sync::Arc;

    use opencoder_cli::run::apply_prompt_file;
    use opencoder_core::{resolve_agent, AgentKind, Config};
    use opencoder_llm::{ChatStream, MockChatClient};
    use opencoder_session::SessionState;

    /// Real (never-run) session for `agent_name`, mirroring the fresh-session
    /// construction in `run_headless`: resolve_agent + SessionState::new with
    /// a mock stream. No store, no network — the seam under test only touches
    /// `session.agent.prompt`.
    fn fresh_session(id: &str, agent_name: &str) -> SessionState {
        let agent = resolve_agent(agent_name).unwrap();
        let config = Config::default();
        SessionState::new(
            id,
            agent,
            config,
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::env::temp_dir(),
        )
    }

    /// Write `body` to a unique temp file (pid + test name suffix) and remove
    /// it again once `f` has consumed the path, so cleanup happens even when
    /// `f` panics.
    fn with_prompt_file<R>(test_name: &str, body: &str, f: impl FnOnce(&Path) -> R) -> R {
        let path = std::env::temp_dir().join(format!(
            "opencoder-prompt-file-{}-{test_name}.txt",
            std::process::id()
        ));
        std::fs::write(&path, body).unwrap();
        let out = f(&path);
        let _ = std::fs::remove_file(&path);
        out
    }

    #[test]
    fn run_with_plan_agent_prompt_file_stores_prompt_without_build_ad() {
        let body = "Review the repository layout and draft a migration plan.\n";
        with_prompt_file("plan-seam", body, |path| {
            let mut session = fresh_session("pf-plan-seam", "plan");

            // The real headless assignment path: read file → compose → store.
            apply_prompt_file(&mut session, path).unwrap();

            let stored = session.agent.prompt;
            // The 'build' delegation ad is stripped in plan mode…
            assert!(
                !stored.contains(opencoder_core::BUILD_DELEGATION_CLAUSE),
                "plan prompt must not advertise build delegation, got:\n{stored}"
            );
            assert!(
                !stored.contains("'build'"),
                "plan prompt must not mention 'build' at all, got:\n{stored}"
            );
            // …but this is a surgical strip, not a preamble wipe: the user's
            // own body and the rest of the tool preamble survive.
            assert!(
                stored.starts_with(body.trim()),
                "user body must survive composition, got:\n{stored}"
            );
            assert!(
                stored.contains("## Tools"),
                "remaining tool preamble must survive, got:\n{stored}"
            );
            // The plan agent selection itself is what drove the strip.
            assert_eq!(session.agent.kind, AgentKind::Plan);
        });
    }

    #[test]
    fn run_with_act_agent_prompt_file_stores_full_preamble() {
        let body = "Act as reviewer for this diff.\n";
        with_prompt_file("act-seam", body, |path| {
            let mut session = fresh_session("pf-act-seam", "act");

            apply_prompt_file(&mut session, path).unwrap();

            let stored = session.agent.prompt;
            // Act keeps the full preamble, 'build' delegation ad included.
            assert!(
                stored.contains(opencoder_core::BUILD_DELEGATION_CLAUSE),
                "act prompt must keep the build delegation clause, got:\n{stored}"
            );
            assert!(
                stored.starts_with(body.trim()),
                "user body must survive composition, got:\n{stored}"
            );
            assert!(
                stored.contains("## Tools"),
                "tool preamble must survive, got:\n{stored}"
            );
            assert_eq!(session.agent.kind, AgentKind::Act);
        });
    }

    #[test]
    fn prompt_file_read_failure_surfaces_file_in_error() {
        // The seam owns the fs read too, so a missing file must fail with the
        // flag spelled out (same message shape the headless entry produced
        // while the read was still inline in run_headless).
        let missing = std::env::temp_dir().join(format!(
            "opencoder-prompt-file-{}-missing-seam.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);
        let mut session = fresh_session("pf-missing-seam", "plan");
        let err = apply_prompt_file(&mut session, &missing).unwrap_err();
        assert!(
            err.to_string().contains("--prompt-file"),
            "error must name the flag, got: {err}"
        );
        // The failed read leaves the resolved agent untouched on the session.
        assert_eq!(session.agent.name, "plan");
    }
}
