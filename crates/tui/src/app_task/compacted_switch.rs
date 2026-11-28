//! Compacted-session load through the real `/task` switch loader. The store
//! level already pins `load_messages_after`; this closes the gap to the
//! switch entry: with `summary_seq` set, `load_session_for_switch` must read
//! ONLY the tail after the folded head, rebuild the synthetic summary from the
//! persisted summary text + images in front of it, and the replayed transcript
//! must open on the Compaction block.

use super::*;
use opencoder_core::{ContentBlock, Role};
use opencoder_session::compaction::SUMMARY_PREFIX;
use opencoder_store::{SessionMeta, TsRegistry, TursoStore};

const SUMMARY: &str = "folded: earlier turns summarized";
const SUMMARY_IMAGE: &str = "file:///folded/kept.png";
const SESSION: &str = "compacted";

fn msg(id: &str, role: Role, text: &str) -> Message {
    let mut message = Message::user(id, text);
    message.role = role;
    message
}

async fn session_ids(store: &Arc<dyn Store>) -> Vec<String> {
    let sessions = store
        .list_sessions(&opencoder_store::SessionFilter::default())
        .await
        .unwrap();
    let mut ids: Vec<String> = sessions.into_iter().map(|s| s.id).collect();
    ids.sort();
    ids
}

/// Seed a compacted session behind the production ts-mirror wrapper (the
/// store the TUI runs on once a registry exists), with an unreadable folded
/// head: only the OFFSET tail read may touch this transcript. Returns the
/// wrapped store plus the raw inner store for row-level checks.
async fn compacted_store(dir: &tempfile::TempDir) -> (Arc<dyn Store>, Arc<TursoStore>) {
    let inner = Arc::new(TursoStore::open_memory().await.unwrap());
    let ts_db = dir.path().join("ts.db");
    TsRegistry::open(&ts_db).await.unwrap();
    assert!(
        ts_db.is_file(),
        "fixture: the registry file installs the mirror"
    );
    let store = crate::ts_mirror::maybe_wrap_at(inner.clone(), dir.path(), ts_db).await;
    store
        .create_session(&SessionMeta {
            id: SESSION.into(),
            agent: Some("act".into()),
            model: Some("shared/compacted".into()),
            summary: Some(SUMMARY.into()),
            summary_seq: Some(2),
            summary_images: vec![SUMMARY_IMAGE.into()],
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            SESSION,
            &[
                msg("head-1", Role::User, "folded question"),
                msg("head-2", Role::Assistant, "folded answer"),
            ],
        )
        .await
        .unwrap();
    store
        .append_messages(
            SESSION,
            &[
                msg("tail-1", Role::User, "kept question"),
                msg("tail-2", Role::Assistant, "kept answer"),
            ],
        )
        .await
        .unwrap();
    // The head rows are never supposed to be decoded again; a broken
    // provider-state column turns any full load into a hard error.
    let conn = inner.conn().await.unwrap();
    conn.execute(
        "UPDATE messages SET provider_state_json = '{' WHERE session_id = ? AND seq <= 2",
        (SESSION,),
    )
    .await
    .unwrap();
    drop(conn);
    let full = match store.load_messages(SESSION).await {
        Ok(messages) => panic!("the folded head must be unreadable: {messages:?}"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        full.contains("provider_state_json"),
        "fixture: the head rows fail a full load: {full}"
    );
    (store, inner)
}

#[tokio::test]
async fn load_session_for_switch_on_compacted_session_loads_summary_then_offset_tail() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let (store, inner) = compacted_store(&dir).await;
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());

    let (session, pending) =
        load_session_for_switch(&store, SESSION, Config::default(), &client, dir.path())
            .await
            .unwrap();
    assert_eq!(pending, 0, "no subagent task exists");
    assert_eq!(session.id, SESSION);
    assert_eq!(session.summary.as_deref(), Some(SUMMARY));
    assert_eq!(session.summary_seq, Some(2));
    assert_eq!(session.summary_images, vec![SUMMARY_IMAGE.to_string()]);

    // Synthetic summary first: the persisted text under the marker prefix,
    // followed by the persisted image.
    let [summary, tail @ ..] = session.messages.as_slice() else {
        panic!(
            "summary plus tail expected, got {} messages",
            session.messages.len()
        );
    };
    assert!(summary.synthetic, "the summary is a synthetic message");
    assert_eq!(summary.role, Role::User);
    assert!(
        summary.text().starts_with(SUMMARY_PREFIX),
        "the marker prefixes the persisted summary: {}",
        summary.text()
    );
    assert!(summary.text().contains(SUMMARY));
    assert!(
        summary
            .blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::Image { url, .. } if url == SUMMARY_IMAGE)),
        "persisted summary images ride on the summary message"
    );

    // Only the post-compaction tail: the folded head is never loaded.
    let ids: Vec<&str> = tail.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["tail-1", "tail-2"], "OFFSET tail only");
    assert!(
        session
            .messages
            .iter()
            .all(|m| !m.text().contains("folded ")),
        "no folded head content may come back"
    );

    // The stored transcript is a pure read: nothing is written back, and
    // the folded head rows are still in place behind the OFFSET. The head
    // cannot be decoded (by design), so its presence is counted on the raw
    // rows of the inner store.
    assert_eq!(session_ids(&store).await, vec![SESSION.to_string()]);
    assert_eq!(
        store.load_messages_after(SESSION, 2).await.unwrap().len(),
        2,
        "the OFFSET tail still holds exactly the two kept rows"
    );
    let conn = inner.conn().await.unwrap();
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?",
            (SESSION,),
        )
        .await
        .unwrap();
    let stored: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
    assert_eq!(stored, 4, "head + tail rows must all survive the load");

    // Replaying the loaded state opens on the Compaction block, then the
    // continued turns.
    let chat =
        crate::session_ui::replay_into_chat("act", &session.messages, &store, SESSION, 0).await;
    match chat.blocks.first() {
        Some(crate::chat::ChatBlock::Compaction {
            text,
            collapsed,
            streaming,
        }) => {
            assert!(text.contains(SUMMARY), "compaction text: {text}");
            assert!(*collapsed && !streaming, "a settled, collapsed block");
        }
        other => panic!("the transcript must open on the Compaction block: {other:?}"),
    }
    let rendered = crate::chat::block_text(&chat);
    assert!(rendered.contains("kept question"), "{rendered}");
    assert!(rendered.contains("kept answer"), "{rendered}");
    assert!(
        !rendered.contains("folded question") && !rendered.contains("folded answer"),
        "folded head must not render: {rendered}"
    );
}
