//! `/task` (and `session list`) ordering: sessions sort by **recent activity**
//! (`MAX(updated_at, created_at)`), not creation time, and keyset cursors
//! follow the same key. Appending a message IS activity and touches
//! `updated_at` monotonically inside the append transaction; bulk `import`
//! deliberately does not (history backfill must not masquerade as activity).

use opencoder_core::Message;
use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, Store, TASK_TYPE_PARENT};

async fn mem() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

fn meta(id: &str, created_at: i64, updated_at: i64) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        model: None,
        autopilot_mode: None,
        workdir_hash: None,
        created_at,
        updated_at,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: Some(TASK_TYPE_PARENT.into()),
        requirement: None,
    }
}

fn msg(text: &str, created_at: i64) -> Message {
    let mut m = Message::user(format!("m-{text}-{created_at}"), text);
    m.created_at = created_at;
    m
}

async fn seed(store: &LibsqlStore) {
    // Activity order (desc): s_hot(9_000) > s_mid(5_000) > s_old(1_000) >
    // s_never(300: updated_at=0 -> falls back to created_at) > s_import(100).
    store
        .create_session(&meta("s_hot", 9_000, 9_000))
        .await
        .unwrap();
    store
        .create_session(&meta("s_mid", 1_500, 5_000))
        .await
        .unwrap();
    store
        .create_session(&meta("s_old", 1_000, 1_000))
        .await
        .unwrap();
    store
        .create_session(&meta("s_never", 300, 0))
        .await
        .unwrap();
    store
        .create_session(&meta("s_import", 100, 0))
        .await
        .unwrap();
}

async fn list_ids(store: &LibsqlStore, filter: &SessionFilter) -> Vec<String> {
    store
        .list_sessions(filter)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect()
}

#[tokio::test]
async fn orders_by_recent_activity_not_creation() {
    let store = mem().await;
    seed(&store).await;
    let ids = list_ids(&store, &SessionFilter::default()).await;
    assert_eq!(
        ids,
        vec!["s_hot", "s_mid", "s_old", "s_never", "s_import"],
        "activity desc; updated_at=0 rows fall back to created_at"
    );
}

#[tokio::test]
async fn append_message_bumps_activity_monotonically() {
    let store = mem().await;
    store
        .create_session(&meta("s1", 1_000, 1_000))
        .await
        .unwrap();
    store
        .create_session(&meta("s2", 2_000, 2_000))
        .await
        .unwrap();
    // s2 leads before any traffic.
    assert_eq!(
        list_ids(&store, &SessionFilter::default()).await,
        vec!["s2", "s1"]
    );
    // A new message on s1 floats it above s2.
    store
        .append_message("s1", &msg("hello", 5_000))
        .await
        .unwrap();
    assert_eq!(
        list_ids(&store, &SessionFilter::default()).await,
        vec!["s1", "s2"]
    );
    // Out-of-order (older) backfill must NOT regress the activity stamp.
    store
        .append_message("s1", &msg("late backfill", 2_500))
        .await
        .unwrap();
    assert_eq!(
        list_ids(&store, &SessionFilter::default()).await,
        vec!["s1", "s2"],
        "MAX(updated_at, msg ts) keeps the stamp monotonic"
    );
    let item = &store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap()[0];
    assert_eq!(item.updated_at, 5_000);
}

#[tokio::test]
async fn cursor_pagination_follows_activity_order() {
    let store = mem().await;
    seed(&store).await;
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let filter = SessionFilter {
            limit: 2,
            cursor: cursor.clone(),
            ..SessionFilter::default()
        };
        let items = store.list_sessions(&filter).await.unwrap();
        if items.is_empty() {
            break;
        }
        let last = items.last().unwrap();
        cursor = Some(format!(
            "{}|{}",
            last.updated_at.max(last.created_at),
            last.id
        ));
        seen.extend(items.into_iter().map(|s| s.id));
    }
    assert_eq!(
        seen,
        vec!["s_hot", "s_mid", "s_old", "s_never", "s_import"],
        "keyset pages tile the full activity order without dupes or gaps"
    );
}

#[tokio::test]
async fn invalid_cursor_is_an_error() {
    let store = mem().await;
    seed(&store).await;
    let filter = SessionFilter {
        cursor: Some("not-a-cursor".into()),
        ..SessionFilter::default()
    };
    assert!(store.list_sessions(&filter).await.is_err());
}

#[tokio::test]
async fn import_does_not_masquerade_as_activity() {
    let store = mem().await;
    store
        .create_session(&meta("live", 1_000, 1_000))
        .await
        .unwrap();
    store
        .create_session(&meta("imported", 100, 0))
        .await
        .unwrap();
    // Bulk history load onto the imported session: high message timestamps,
    // but this is a backfill, not user activity.
    store
        .import_messages("imported", &[msg("old history", 99_999)])
        .await
        .unwrap();
    assert_eq!(
        list_ids(&store, &SessionFilter::default()).await,
        vec!["live", "imported"],
        "import must not float a backfilled session to the top"
    );
}
