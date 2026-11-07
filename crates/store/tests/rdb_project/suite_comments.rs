//! Comment pagination, append idempotency and execution-source checks
//! (ported from the SQL comment suite; schema-migration cases stay SQL-only).
use super::*;
use opencoder_store::project::comments::{CommentError, CommentQuery, NewComment};

async fn setup() -> Arc<dyn ProjectStore> {
    let s = mem_iface().await;
    s.create_todo(&todo("t", None, 1)).await.unwrap();
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
    // Default page: newest first, 50 rows, more behind it.
    let first = s
        .todo_comments("t", CommentQuery::default())
        .await
        .unwrap()
        .unwrap();
    assert!(first.has_more);
    assert_eq!(first.comments.len(), 50);
    assert_eq!(first.comments[0].entry.request_id, "r-55");
    // Walking `before` reaches the oldest row exactly.
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
    // Appends after the first page show up through `after`.
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
    // Unknown todo: None (the page projection requires the row).
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
    // A different body under the same request identity is a conflict.
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
    // Registry names and timestamps may change between retries; input cannot.
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
    // Deleting the todo removes the comment identity with it.
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
    // No execution link yet: rejected.
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
    // The capability author must match the linked execution's capability.
    entry.author_kind = "capability".into();
    entry.author_id = "different".into();
    assert!(s.append_todo_comment(&entry).await.is_err());
    entry.author_id = "cap".into();
    let comment = s.append_todo_comment(&entry).await.unwrap();
    // Unlinking afterwards keeps the stored comment, and the same request
    // replays idempotently against the stored row.
    s.unlink_todo_execution("t", "agent-source").await.unwrap();
    assert_eq!(
        s.append_todo_comment(&entry).await.unwrap().seq,
        comment.seq
    );
    // A fresh request against the missing source is rejected again.
    entry.request_id = "new".into();
    assert!(s.append_todo_comment(&entry).await.is_err());
}
