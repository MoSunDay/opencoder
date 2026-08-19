//! Integration tests for the autopilot loop's persisted-skill lifecycle:
//! the system-injected review skill must not leak into the store when the
//! loop ends, regardless of outcome. Loop mechanics live in `autopilot.rs`;
//! the one-shot review pass lives in `autopilot_review.rs`.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, ApMode, AutoPilotConfig, Config, Message};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::autopilot::drive;
use opencoder_session::tools::registry;
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

/// A completed turn with optional tool calls (empty tools = idle/Done).
fn completed(text: &str, tool_calls: Vec<CompletedToolCall>) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls,
        usage: Some(Usage::default()),
    }
}

fn autopilot_config(max_iterations: u32, verify_retries: u32) -> Config {
    Config {
        model: "m/g".into(),
        autopilot: AutoPilotConfig {
            mode: ApMode::Ap,
            max_iterations,
            verify_retries,
            ..AutoPilotConfig::default()
        },
        ..Config::default()
    }
}

fn make_session(mock: Arc<dyn ChatStream>, config: Config) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new("ap-sess", agent, config, mock, dir.path().to_path_buf());
    (dir, s)
}

fn collector() -> (Arc<Mutex<Vec<SessionEvent>>>, impl FnMut(SessionEvent)) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let clone = buf.clone();
    let f = move |ev: SessionEvent| clone.lock().unwrap().push(ev);
    (buf, f)
}

/// The system-injected review skill must not leak into the store when the
/// autopilot loop ends — on EITHER outcome. Two scenarios share one shape:
/// (a) Complete: drive finishes normally; (b) phase error: iteration 1's
/// PLAN hits the exhausted mock. Both must leave `sessions.skill` NULL, or
/// a resume resurrects a skill the user never picked.
#[tokio::test]
async fn drive_clears_persisted_skill_on_complete_and_on_error() {
    for (label, scripts, expect_err) in [
        (
            "complete",
            vec![
                completed("plan-0", vec![]),
                completed("act-0", vec![]),
                completed("yes", vec![]), // VERIFY -> Complete
            ],
            false,
        ),
        (
            "phase-error",
            vec![
                completed("plan-0", vec![]),
                completed("act-0", vec![]),
                completed("no", vec![]), // MoreWork -> iteration 1
                                         // iteration 1 PLAN: mock exhausted -> error
            ],
            true,
        ),
    ] {
        let mut builder = MockChatClient::new();
        for s in scripts {
            builder = builder.push_script(vec![s]);
        }
        let mock = Arc::new(builder) as Arc<dyn ChatStream>;
        let (_dir, mut session) = make_session(mock, autopilot_config(10, 3));

        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "ap-sess".into(),
                agent: Some("act".into()),
                skill: Some("STALE-REVIEW-BODY".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        session.store = Some(store.clone());
        session
            .record(Message::user("u1", "implement feature X"))
            .await;

        let reg = registry();
        let (_buf, mut on_event) = collector();
        let res = drive(&mut session, &reg, &mut on_event).await;
        assert_eq!(
            res.is_err(),
            expect_err,
            "{label}: unexpected outcome {res:?}"
        );
        assert!(
            session.skill_prompt_cloned().is_none(),
            "{label}: in-memory skill must be cleared"
        );
        let stored = store
            .get_session("ap-sess")
            .await
            .unwrap()
            .expect("session row exists");
        assert!(
            stored.skill.is_none(),
            "{label}: the clear must be persisted (clear_skill), got {:?}",
            stored.skill
        );
    }
}
