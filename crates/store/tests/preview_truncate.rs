//! `sessions::list` preview subquery caps the pulled `blocks_json` at 8 KB
//! (`substr(m.blocks_json, 1, 8192)`), so a first user message carrying MB-scale
//! base64 images no longer drags the whole blob through the list query. A
//! truncated payload can parse as invalid JSON; `extract_preview` must degrade
//! to an empty preview instead of erroring the listing.

use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, Store};

async fn mem() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        model: Some("m".into()),
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

fn user_msg(id: &str, blocks: Vec<ContentBlock>) -> Message {
    Message {
        id: id.into(),
        role: Role::User,
        blocks,
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

async fn first_preview(store: &LibsqlStore) -> String {
    let items = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    items
        .first()
        .map(|i| i.preview.clone())
        .unwrap_or_else(|| panic!("session must be listed"))
}

/// A compact text-first user message stays within the 8 KB cap, so the preview
/// still extracts the leading text exactly as before the cap.
#[tokio::test]
async fn preview_extracts_text_within_cap() {
    let store = mem().await;
    store.create_session(&meta("p1")).await.unwrap();
    let msgs = vec![user_msg(
        "m1",
        vec![ContentBlock::text("fix the login bug please")],
    )];
    store.append_messages("p1", &msgs).await.unwrap();

    let preview = first_preview(&store).await;
    assert!(
        preview.contains("fix the login bug"),
        "text within cap must still preview; got {preview:?}"
    );
}

/// Image-first blocks whose base64 overflows the 8 KB cap truncate the JSON
/// mid-string; the parse fails and the preview degrades to empty while the
/// listing itself keeps succeeding.
#[tokio::test]
async fn preview_degrades_when_image_first_overflows_cap() {
    let store = mem().await;
    store.create_session(&meta("p2")).await.unwrap();
    let huge_b64 = "A".repeat(64 * 1024);
    let msgs = vec![user_msg(
        "m1",
        vec![
            ContentBlock::Image {
                url: format!("data:image/png;base64,{huge_b64}"),
                detail: None,
            },
            ContentBlock::text("screenshot of the crash"),
        ],
    )];
    store.append_messages("p2", &msgs).await.unwrap();

    let preview = first_preview(&store).await;
    assert!(
        preview.is_empty(),
        "truncated JSON must degrade to empty preview; got {preview:?}"
    );
}

/// Text-first but longer than the cap truncates inside the string value; same
/// safe degradation to empty.
#[tokio::test]
async fn preview_degrades_when_text_exceeds_cap() {
    let store = mem().await;
    store.create_session(&meta("p3")).await.unwrap();
    let long = "x".repeat(20 * 1024);
    let msgs = vec![user_msg("m1", vec![ContentBlock::text(long)])];
    store.append_messages("p3", &msgs).await.unwrap();

    let preview = first_preview(&store).await;
    assert!(
        preview.is_empty(),
        "over-cap text must degrade to empty preview; got {preview:?}"
    );
}
