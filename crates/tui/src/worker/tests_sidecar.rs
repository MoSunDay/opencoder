//! Sidecar persistence-gate tests for `persist_event`: the three
//! `Sidecar*` display frames must never land in the store, while the bare
//! `LlmUsage` the sidecar actor forwards MUST (cost accounting).

use super::*;

/// Fresh in-memory store with the session row created: `session_events`
/// carries a `REFERENCES sessions(id)` foreign key, so the session must
/// exist before `persist_event` can append.
async fn memory_store(sid: &str) -> Arc<dyn Store> {
    let store: Arc<dyn Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    store
}

async fn stored_sse_kinds(store: &Arc<dyn Store>, session_id: &str) -> Vec<String> {
    store
        .events_after(session_id, 0)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.sse_kind.unwrap_or_default())
        .collect()
}

/// The three sidecar frames are display-only: `persist_event` drops them
/// before the store is ever touched.
#[tokio::test]
async fn sidecar_frames_are_never_persisted() {
    let sid = "sidecar-gate";
    let store = memory_store(sid).await;
    let frames: Vec<SessionEvent> = vec![
        SessionEvent::SidecarStart {
            id: "sc-1".into(),
            question: "q".into(),
        },
        SessionEvent::SidecarChild {
            id: "sc-1".into(),
            ev: Box::new(SessionEvent::TextDelta("旁路内容".into())),
        },
        SessionEvent::SidecarTurn {
            id: "sc-1".into(),
            ok: true,
            answer: "答案".into(),
            elapsed_ms: 5,
            total_tokens: 9,
            rounds: 1,
        },
    ];
    for f in &frames {
        assert!(f.is_sidecar_frame());
        persist_event(&Some(store.clone()), sid, f).await;
    }
    let kinds = stored_sse_kinds(&store, sid).await;
    assert!(
        kinds.is_empty(),
        "sidecar frames must not reach the store, got {kinds:?}"
    );
}

/// The actor's bare `LlmUsage` forward persists like any main-task round:
/// this is how sidecar cost reaches the durable layer (web replay).
#[tokio::test]
async fn bare_sidecar_llm_usage_is_persisted() {
    let sid = "sidecar-cost";
    let store = memory_store(sid).await;
    persist_event(
        &Some(store.clone()),
        sid,
        &SessionEvent::LlmUsage {
            total_tokens: 321,
            input_tokens: 300,
            output_tokens: 21,
        },
    )
    .await;
    let kinds = stored_sse_kinds(&store, sid).await;
    assert_eq!(
        kinds,
        vec!["llm_usage".to_string()],
        "exactly the bare usage row lands"
    );
}

/// Non-sidecar traffic is unaffected by the gate (sanity: normal frames
/// still persist through the same function).
#[tokio::test]
async fn ordinary_frames_still_persist() {
    let sid = "sidecar-ordinary";
    let store = memory_store(sid).await;
    persist_event(
        &Some(store.clone()),
        sid,
        &SessionEvent::Status("running".into()),
    )
    .await;
    let kinds = stored_sse_kinds(&store, sid).await;
    assert_eq!(kinds.len(), 1, "got {kinds:?}");
}
