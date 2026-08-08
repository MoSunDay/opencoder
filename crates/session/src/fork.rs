//! Shared session-fork logic: copy a session's meta + messages into a brand
//! new session id, leaving the original untouched. Single implementation used
//! by the CLI (`--fork`), the Web API (`POST /api/sessions/:id/fork`) and the
//! TUI `/fork` picker. Callers own their messaging (CLI prints a fork notice).

use anyhow::{anyhow, Result};
use opencoder_core::message::now_ms;
use opencoder_store::{SessionMeta, Store};

use crate::runner::new_id;

/// Copy a session's meta and messages into a new session id, leaving the
/// original untouched. Returns the new id.
pub async fn fork_session(store: &dyn Store, parent_id: &str) -> Result<String> {
    let meta = store
        .get_session(parent_id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {parent_id}"))?;
    let messages = store.load_messages(parent_id).await?;
    let new_id = new_id();
    let now = now_ms();
    let forked = SessionMeta {
        id: new_id.clone(),
        title: meta.title.as_deref().map(|t| format!("{t} (fork)")),
        agent: meta.agent.clone(),
        model: meta.model.clone(),
        workdir_hash: meta.workdir_hash.clone(),
        created_at: now,
        updated_at: now,
        summary: meta.summary.clone(),
        summary_seq: meta.summary_seq,
        summary_images: vec![],
        handoff_seq: meta.handoff_seq,
        handoff_plan: meta.handoff_plan.clone(),
        skill: meta.skill.clone(),
        task_type: None,
        requirement: None,
    };
    store.create_session(&forked).await?;
    if !messages.is_empty() {
        store.append_messages(&new_id, &messages).await?;
    }
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::{ContentBlock, Message};
    use opencoder_store::LibsqlStore;

    fn assistant_with_text(id: &str, text: &str) -> Message {
        let mut m = Message::assistant(id);
        m.blocks.push(ContentBlock::text(text));
        m
    }

    async fn seed(store: &dyn Store, id: &str, task_type: Option<&str>) {
        store
            .create_session(&SessionMeta {
                id: id.into(),
                title: Some("parent".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec!["img:legacy".into()],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: task_type.map(String::from),
                requirement: None,
            })
            .await
            .unwrap();
        store
            .append_message(id, &Message::user("u1", "hello"))
            .await
            .unwrap();
        store
            .append_message(id, &assistant_with_text("a1", "world"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn fork_copies_messages_and_resets_meta() {
        let store = LibsqlStore::open_memory().await.unwrap();
        // Seed with a non-default task_type to prove the fork resets it.
        seed(&store, "parent", Some("subagent")).await;

        let child_id = fork_session(&store, "parent").await.unwrap();
        assert_ne!(child_id, "parent", "fork must create a new id");

        let parent_msgs = store.load_messages("parent").await.unwrap();
        let child_msgs = store.load_messages(&child_id).await.unwrap();
        assert_eq!(parent_msgs.len(), 2, "parent unchanged");
        assert_eq!(child_msgs.len(), 2, "child has same message count");
        assert_eq!(child_msgs[0].text(), "hello");
        assert_eq!(child_msgs[1].text(), "world");

        let child = store.get_session(&child_id).await.unwrap().unwrap();
        assert_eq!(child.title.as_deref(), Some("parent (fork)"));
        assert_eq!(
            child.task_type.as_deref(),
            Some("parent"),
            "fork resets task_type to the fresh-session default, not the parent's"
        );
        assert!(
            child.summary_images.is_empty(),
            "fork resets summary_images"
        );
        assert_eq!(child.model.as_deref(), Some("m"), "model carried over");
    }

    #[tokio::test]
    async fn fork_nonexistent_session_errors() {
        let store = LibsqlStore::open_memory().await.unwrap();
        let err = fork_session(&store, "ghost").await;
        assert!(err.is_err(), "forking a nonexistent session should fail");
    }
}
