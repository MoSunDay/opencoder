//! Regression for the "reminder on every turn" bug (run-end skill contract
//! on the early-exit branch): a crash mid-run can leave a skill armed in
//! memory AND persisted in the store row, and a bare control command
//! (`/plan`, `/act`) returns BEFORE the one-shot wrapper. Without the
//! explicit run-end clear in that early-exit branch, the skill would
//! survive the bare command and resurrect into every later run (tail
//! reminder + latent unlocks each turn). Both halves must be cleared.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Armed skill (memory + store row) + bare control command -> run ends
/// without an LLM call and the skill is cleared from BOTH memory and the
/// store row, so no later resume can resurrect it.
#[tokio::test]
async fn bare_control_cmd_clears_armed_skill_in_memory_and_store() {
    let store = mem_store().await;
    seed(&store, "bare-cmd-skill", "act").await;
    let skill_body = "> Source: /skills/rev/SKILL.md\n\nREV";

    // Simulate the crash-resume state: the store row carries the skill.
    store
        .update_session(
            "bare-cmd-skill",
            &SessionPatch {
                skill: Some(skill_body.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "bare-cmd-skill",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created()
    .with_skill(skill_body.into());
    assert!(session.skill_prompt_cloned().is_some(), "armed up front");

    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev_clone = events.clone();
    run(&mut session, "/plan".into(), move |ev| {
        ev_clone.lock().unwrap().push(ev)
    })
    .await
    .unwrap();

    {
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(e, SessionEvent::Done)));
        assert!(
            !evs.iter().any(|e| matches!(e, SessionEvent::TextDelta(_))),
            "bare control command makes no LLM call"
        );
    }
    assert!(
        session.skill_prompt_cloned().is_none(),
        "in-memory skill must be cleared by the early-exit branch"
    );
    let meta = store.get_session("bare-cmd-skill").await.unwrap().unwrap();
    assert!(
        meta.skill.is_none(),
        "store row must be cleared too, or resume resurrects the skill"
    );
}
