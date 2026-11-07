//! Bundle-assembly unit tests for `rows.rs`: root detection, nesting order,
//! counts, dangling references and cycle rejection.

use super::{session, text_message};
use crate::legacy::rows::{assemble_roots, LegacyDataset, LegacyMessage, LegacyTask};

fn dataset_with_subagent() -> LegacyDataset {
    let mut ds = LegacyDataset {
        sessions: vec![session("root", 1), session("child", 2)],
        ..Default::default()
    };
    ds.messages = vec![
        LegacyMessage {
            session_id: "root".into(),
            message: text_message("m1", "hello"),
        },
        LegacyMessage {
            session_id: "child".into(),
            message: text_message("m2", "deep"),
        },
    ];
    ds.subagent_tasks = vec![LegacyTask {
        task: opencoder_store::SubagentTaskRecord {
            task_id: "t1".into(),
            parent_session_id: "root".into(),
            child_session_id: "child".into(),
            parent_message_id: Some("m1".into()),
            agent: "explore".into(),
            prompt: "go".into(),
            result: Some("done".into()),
            status: opencoder_store::SubagentStatus::Completed,
            ok: Some(true),
            started_at: 2,
            completed_at: Some(3),
        },
    }];
    ds
}

#[test]
fn assembly_builds_one_root_with_nested_child() {
    let forest = assemble_roots(dataset_with_subagent()).unwrap();
    assert_eq!(forest.roots.len(), 1, "child is nested, not a root");
    let root = &forest.roots[0];
    assert_eq!(root.meta.id, "root");
    assert_eq!(root.messages.len(), 1);
    assert_eq!(root.subagents.len(), 1);
    assert_eq!(root.subagents[0].child.meta.id, "child");
    assert_eq!(root.subagents[0].child.messages.len(), 1);
    assert_eq!(forest.counts.roots, 1);
    assert_eq!(forest.counts.sessions, 2);
    assert_eq!(forest.counts.messages, 2);
    assert_eq!(forest.counts.subagent_tasks, 1);
}

#[test]
fn assembly_errors_on_dangling_references_and_cycles() {
    let mut ds = dataset_with_subagent();
    ds.sessions.retain(|s| s.meta.id != "child");
    let err = assemble_roots(ds).unwrap_err();
    assert!(
        err.to_string().contains("missing child session"),
        "dangling child reported: {err}"
    );

    let mut ds = dataset_with_subagent();
    ds.sessions.retain(|s| s.meta.id != "root");
    let err = assemble_roots(ds).unwrap_err();
    assert!(
        err.to_string().contains("missing parent session"),
        "dangling parent reported: {err}"
    );

    // Two tasks pointing at each other's sessions: neither is a root and the
    // recursion guard must fire instead of looping.
    let mut ds = LegacyDataset {
        sessions: vec![session("a", 1), session("b", 2)],
        ..Default::default()
    };
    let task = |parent: &str, child: &str| LegacyTask {
        task: opencoder_store::SubagentTaskRecord {
            task_id: format!("t-{parent}"),
            parent_session_id: parent.into(),
            child_session_id: child.into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "go".into(),
            result: None,
            status: opencoder_store::SubagentStatus::Running,
            ok: None,
            started_at: 1,
            completed_at: None,
        },
    };
    ds.subagent_tasks = vec![task("a", "b"), task("b", "a")];
    assert!(assemble_roots(ds).is_err(), "cyclic references must error");
}

#[test]
fn assembly_orders_roots_by_creation_then_id() {
    let ds = LegacyDataset {
        sessions: vec![session("z", 1), session("b", 0), session("a", 0)],
        ..Default::default()
    };
    let forest = assemble_roots(ds).unwrap();
    let ids: Vec<&str> = forest.roots.iter().map(|b| b.meta.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "z"]);
}
