//! Multimodal input persistence: image data URIs attached to a `SessionInput`
//! survive admit / pending / claim at the libsql layer (the `images_json`
//! column added in schema v4).

use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn make_session(store: &LibsqlStore, id: &str) {
    let now = 1;
    let meta = SessionMeta {
        id: id.to_string(),
        title: None,
        agent: Some("act".into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
    };
    store.create_session(&meta).await.unwrap();
}

#[tokio::test]
async fn images_roundtrip_through_pending_inputs() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s").await;

    let input = SessionInput {
        seq: None,
        id: "in-1".into(),
        session_id: "s".into(),
        delivery: Delivery::Steer,
        prompt: "describe this".into(),
        images: vec![
            "data:image/png;base64,iVBORw0KGgo=".into(),
            "https://x.test/b.jpg".into(),
        ],
        admitted_seq: 1,
        promoted_seq: None,
    };
    store.admit_input(&input).await.unwrap();

    let pending = store
        .pending_inputs("s", Delivery::Steer)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].images,
        vec![
            "data:image/png;base64,iVBORw0KGgo=",
            "https://x.test/b.jpg"
        ],
        "images must round-trip through pending_inputs"
    );
    assert_eq!(pending[0].prompt, "describe this");
}

#[tokio::test]
async fn images_roundtrip_through_claim_next_queue() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s").await;

    let input = SessionInput {
        seq: None,
        id: "q-1".into(),
        session_id: "s".into(),
        delivery: Delivery::Queue,
        prompt: "follow up".into(),
        images: vec!["data:image/png;base64,YQ==".into()],
        admitted_seq: 1,
        promoted_seq: None,
    };
    store.admit_input(&input).await.unwrap();

    let claimed = store.claim_next_queue("s").await.unwrap();
    let (_, claimed_input) = claimed.expect("a queued input to be claimed");
    assert_eq!(claimed_input.images, vec!["data:image/png;base64,YQ=="]);
    // Claiming is idempotent-ish: a second claim returns None.
    assert!(store.claim_next_queue("s").await.unwrap().is_none());
}

#[tokio::test]
async fn plain_text_input_has_empty_images() {
    // Backwards compat: an input admitted with no images must read back as
    // an empty images vec (the column defaults to '[]').
    let (_dir, store) = fresh().await;
    make_session(&store, "s").await;

    let input = SessionInput {
        seq: None,
        id: "t-1".into(),
        session_id: "s".into(),
        delivery: Delivery::Steer,
        prompt: "plain".into(),
        images: Vec::new(),
        admitted_seq: 1,
        promoted_seq: None,
    };
    store.admit_input(&input).await.unwrap();

    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].images.is_empty());
}
