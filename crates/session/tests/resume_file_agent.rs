//! A store session row persisted with a file-based custom agent (by name)
//! resumes through the session layer with THAT agent restored: `resume()`
//! resolves the stored name via `resolve_agent`, which resolves file agents
//! exactly like builtins. Cheapest layer that exercises the real mechanics
//! (store row -> meta.agent -> resolve), following the fixture conventions
//! of `legacy_sandbox_agent_resume.rs`.

use std::sync::{Arc, Mutex};

use opencoder_core::agent::set_agents_dir_override;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::{resume, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

/// Serializes tests touching the process-global agents-root override.
static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

/// Minimal resolvable file agent: a private prompt pool `prompts/<name>/v1`
/// (soul only) plus a card referencing it.
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

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

// The std-mutex guard serializes this whole test (agent-dir override is
// process-global) and is deliberately held across awaits.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn file_agent_session_resumes_with_that_agent() {
    let dir = tempfile::tempdir().unwrap();
    let _g = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_agents_dir_override(Some(dir.path().to_path_buf()));
    write_file_agent(dir.path(), "writer", "Writer soul: small diffs.");

    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "file-agent-sess".into(),
            // The persisted file-agent name, exactly as the creating process
            // (run/tui/web) would have written it.
            agent: Some("writer".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            "file-agent-sess",
            &[opencoder_core::Message::user("u1", "earlier writer work")],
        )
        .await
        .unwrap();

    let mock = Arc::new(MockChatClient::new());
    let resumed: SessionState = resume(
        store.clone(),
        "file-agent-sess",
        config(),
        mock as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    // The stored name resolves back to the file agent — not the "act"
    // fallback — with its card's prompt as the primary agent.
    assert_eq!(resumed.agent.name, "writer");
    assert_eq!(resumed.agent.kind, opencoder_core::AgentKind::Act);
    assert!(resumed.agent.is_primary());
    assert!(
        resumed.agent.prompt.contains("Writer soul"),
        "resumed agent must carry the file agent's composed prompt"
    );
    // Preconditions that make the assertion meaningful.
    assert!(resolve_agent("writer").is_some());
    assert!(
        resumed
            .messages
            .iter()
            .any(|m| m.text().contains("earlier")),
        "persisted history survives the resume"
    );
}
