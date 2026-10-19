use opencoder_store::project::comments::{CommentError, CommentQuery, NewComment};
use opencoder_store::{ProjectStore, Store, TursoStore};
use serde_json::json;

async fn setup() -> TursoStore {
    let s = TursoStore::open_memory().await.unwrap();
    s.create_todo(&serde_json::from_value(json!({"id":"t","title":"Task","draft":"body","status":"draft","agent":"act","created_at":1,"updated_at":1})).unwrap()).await.unwrap();
    s
}

fn note(id: i64) -> NewComment {
    NewComment {
        todo_id: "t".into(),
        request_id: format!("r-{id}"),
        body_md: format!("**note {id}**"),
        author_kind: "user".into(),
        author_id: "alice".into(),
        author_name: "alice".into(),
        capability_kind: None,
        submitted_by: "alice".into(),
        execution_id: None,
        created_at: 2,
    }
}

#[tokio::test]
async fn pagination_has_no_gaps_even_for_equal_timestamps_and_concurrent_appends() {
    let s = setup().await;
    for n in 0..105 {
        s.append_todo_comment(&note(n)).await.unwrap();
    }
    let first = s
        .todo_comments("t", CommentQuery::default())
        .await
        .unwrap()
        .unwrap();
    assert!(first.has_more);
    assert_eq!(first.comments.len(), 50);
    assert_eq!(first.comments[0].entry.request_id, "r-55");
    let older = s
        .todo_comments(
            "t",
            CommentQuery {
                before_seq: Some(first.comments[0].seq),
                limit: Some(100),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!older.has_more);
    assert_eq!(older.comments.len(), 55);
    assert_eq!(older.comments[0].entry.request_id, "r-0");
    s.append_todo_comment(&note(106)).await.unwrap();
    let newer = s
        .todo_comments(
            "t",
            CommentQuery {
                after_seq: Some(first.comments.last().unwrap().seq),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(newer.comments.len(), 1);
    assert!(s
        .todo_comments("missing", CommentQuery::default())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn concurrent_retries_are_idempotent_and_conflicts_leave_task_unchanged() {
    let s = setup().await;
    let entry = note(1);
    let (a, b) = tokio::join!(s.append_todo_comment(&entry), s.append_todo_comment(&entry));
    assert_eq!(a.unwrap().seq, b.unwrap().seq);
    let mut changed = entry.clone();
    changed.body_md = "other".into();
    changed.created_at = 10;
    assert!(matches!(
        s.append_todo_comment(&changed)
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(CommentError::Conflict)
    ));
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().updated_at, 2);
    assert_eq!(
        s.get_todo("t").await.unwrap().unwrap().board_status,
        "backlog"
    );
    let mut retry = entry;
    retry.created_at = 10;
    retry.author_name = "renamed".into();
    assert_eq!(
        s.append_todo_comment(&retry)
            .await
            .unwrap()
            .entry
            .author_name,
        "alice"
    );
    s.delete_todo("t").await.unwrap();
    assert!(s
        .todo_comment_request("t", "alice", "r-1")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        s.append_todo_comment(&note(2))
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(CommentError::MissingTodo)
    ));
}

#[tokio::test]
async fn execution_sources_are_verified_in_the_append_transaction() {
    let s = setup().await;
    let mut entry = note(1);
    entry.execution_id = Some("agent-source".into());
    assert!(matches!(
        s.append_todo_comment(&entry)
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(CommentError::InvalidSource)
    ));
    s.link_todo_execution(&opencoder_store::project::ProjectAssignment {
        todo_id: "t".into(),
        execution_id: "agent-source".into(),
        capability_id: Some("cap".into()),
        kind: "agent".into(),
        name: "source".into(),
        created_at: 2,
    })
    .await
    .unwrap();
    entry.author_kind = "capability".into();
    entry.author_id = "different".into();
    assert!(s.append_todo_comment(&entry).await.is_err());
    entry.author_id = "cap".into();
    let comment = s.append_todo_comment(&entry).await.unwrap();
    s.unlink_todo_execution("t", "agent-source").await.unwrap();
    assert_eq!(
        s.append_todo_comment(&entry).await.unwrap().seq,
        comment.seq
    );
    entry.request_id = "new".into();
    assert!(s.append_todo_comment(&entry).await.is_err());
}

#[tokio::test]
async fn version_38_upgrade_preserves_tasks_and_authentication_rows() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("store.db");
    let s = TursoStore::open(&file).await.unwrap();
    s.create_user(
        "migration-user",
        "fixture-token-digest",
        opencoder_core::identity::Role::Editor,
        1,
    )
    .await
    .unwrap();
    s.create_todo(&serde_json::from_value(json!({"id":"t","title":"Task","draft":"body","status":"draft","agent":"act","created_at":1,"updated_at":1})).unwrap()).await.unwrap();
    let conn = s.conn().await.unwrap();
    conn.execute("DROP TRIGGER project_todo_comments_delete", ())
        .await
        .unwrap();
    conn.execute("DROP TABLE project_todo_comments", ())
        .await
        .unwrap();
    conn.execute("UPDATE schema_version SET version=38", ())
        .await
        .unwrap();
    let count: i64 = conn
        .query("SELECT COUNT(*) FROM platform_users", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    drop(conn);
    drop(s);
    let s = TursoStore::open(&file).await.unwrap();
    assert_eq!(s.get_todo("t").await.unwrap().unwrap().draft, "body");
    assert!(s
        .todo_comments("t", CommentQuery::default())
        .await
        .unwrap()
        .unwrap()
        .comments
        .is_empty());
    s.append_todo_comment(&note(1)).await.unwrap();
    let actual: i64 = s
        .conn()
        .await
        .unwrap()
        .query("SELECT COUNT(*) FROM platform_users", ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert_eq!(count, actual);
    let user = s
        .find_user_by_token_hash("fixture-token-digest")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.name, "migration-user");
    assert_eq!(user.role, opencoder_core::identity::Role::Editor);
}
