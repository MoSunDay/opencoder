//! Real worker admission and reads before the first session event exists.
#[path = "../support/mod.rs"]
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_worker::Worker;
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
use support::*;

async fn command(
    node: &Worker,
    id: &str,
    kind: ExecutionKind,
    action: &str,
    input: Value,
) -> RpcReply {
    node.handle(NodeOperation::Command {
        execution: ExecutionRef {
            id: id.into(),
            kind,
        },
        command: ExecutionCommand {
            action: action.into(),
            input,
        },
    })
    .await
}

async fn initial_reads(kind: ExecutionKind, cancel: bool) {
    let _config = isolated_config();
    let root = tempfile::tempdir().unwrap();
    let client = Arc::new(InterruptClient {
        calls: Default::default(),
        first_release: Default::default(),
        resumed_release: Default::default(),
    });
    let node = worker_with_client(root.path(), client.clone()).await;
    let configured = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "configure_scheduling".into(),
                input: json!({"max_runs":1,"queue_order":"fifo"}),
            },
        })
        .await;
    assert_eq!(configured.status, 200);
    let holder = assignment(
        &node,
        "agent-holder",
        ExecutionKind::Agent,
        json!({"prompt":"hold","title":"slot holder"}),
        None,
    );
    assert_eq!(
        node.handle(NodeOperation::Create { assignment: holder })
            .await
            .status,
        200
    );
    let id = format!("{}-initializing", kind.prefix());
    let pending = assignment(
        &node,
        &id,
        kind,
        json!({"prompt":"run after holder","title":"waiting"}),
        None,
    );
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: pending.clone(),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    assert_eq!(accepted.body["status"], "pending");
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: pending
        })
        .await
        .body,
        accepted.body
    );
    for tail in ["", "messages", "seq", "transcript"] {
        let reply = command(
            &node,
            &id,
            kind,
            "http",
            json!({"method":"GET","tail":tail}),
        )
        .await;
        assert_eq!(reply.status, 200, "{tail}: {reply:?}");
        if tail == "seq" {
            assert_eq!(reply.body["seq"], 0);
        } else {
            assert_eq!(reply.body["initializing"], true);
        }
    }
    let summary = command(&node, &id, kind, "summary", Value::Null).await;
    assert_eq!(summary.status, 200);
    assert_eq!(summary.body["id"], id);
    let execution = ExecutionRef {
        id: id.clone(),
        kind,
    };
    let events = node
        .handle(NodeOperation::Events {
            execution: execution.clone(),
            after: 0,
        })
        .await;
    assert_eq!(events.status, 200, "{events:?}");
    assert_eq!(
        events.body["finished"], false,
        "no first event is not a completed execution"
    );
    let messages = node
        .handle(NodeOperation::Messages {
            execution,
            cursor: MessageCursor::default(),
        })
        .await;
    assert_eq!(messages.status, 200, "{messages:?}");
    assert_eq!(messages.body["chunks"], json!([]));
    let unknown = command(
        &node,
        "unknown",
        kind,
        "http",
        json!({"method":"GET","tail":""}),
    )
    .await;
    assert_eq!(unknown.status, 404);
    if cancel {
        let reply = command(&node, &id, kind, "cancel", Value::Null).await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let read = command(&node, &id, kind, "http", json!({"method":"GET","tail":""})).await;
        assert_eq!(read.status, 409, "{read:?}");
        assert!(read.body["error"].as_str().unwrap().contains("cancelled"));
    } else {
        // Reproduce the gap after the durable session row is written but
        // before Harness initialization and the first background drain.
        let store = LibsqlStore::open(root.path().join("node/runtime.db"))
            .await
            .unwrap();
        store
            .create_session(&SessionMeta {
                id: id.clone(),
                kind: Some(kind.prefix().into()),
                agent: Some("act".into()),
                title: Some("waiting".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        for tail in ["", "messages"] {
            let read = command(
                &node,
                &id,
                kind,
                "http",
                json!({"method":"GET","tail":tail}),
            )
            .await;
            assert_eq!(read.status, 200, "{read:?}");
            assert_eq!(
                read.body["draining"], true,
                "accepted execution must remain observable: {read:?}"
            );
            assert_eq!(read.body["messages"], json!([]));
        }
        let events = node
            .handle(NodeOperation::Events {
                execution: ExecutionRef {
                    id: id.clone(),
                    kind,
                },
                after: 0,
            })
            .await;
        assert_eq!(events.status, 200, "{events:?}");
        assert_eq!(
            events.body["finished"], false,
            "a saved session waiting for capacity must keep its event stream open"
        );
        drop(store);
    }
    client.first_release.notify_one();
    settled(&node, "agent-holder").await;
    if !cancel {
        client.resumed_release.notify_one();
        assert_eq!(settled(&node, &id).await["execution"]["status"], "idle");
        let read = command(&node, &id, kind, "http", json!({"method":"GET","tail":""})).await;
        assert_eq!(read.status, 200, "{read:?}");
        assert!(read.body.get("initializing").is_none());
        assert_eq!(read.body["draining"], false);
        assert!(read.body["messages"].to_string().contains("done"));
    }
    assert_eq!(
        client.calls.load(Ordering::SeqCst),
        if cancel { 1 } else { 2 }
    );
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn agent_is_readable_while_waiting_for_a_host_slot() {
    Box::pin(initial_reads(ExecutionKind::Agent, false)).await;
}
// Exercise the native HTTP drain on a normal Tokio worker thread, not the
// larger test-thread stack used by a current-thread runtime.
#[test]
fn operator_is_readable_while_waiting_for_a_host_slot() {
    // Config isolation is thread-local; Tokio threads need their own guard.
    thread_local! {
        static CONFIG: std::cell::RefCell<Option<(
            opencoder_core::config::ScopedConfigHome,
            tempfile::TempDir,
        )>> = const { std::cell::RefCell::new(None) };
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .on_thread_start(|| CONFIG.with(|slot| *slot.borrow_mut() = Some(isolated_config())))
        .on_thread_stop(|| CONFIG.with(|slot| drop(slot.borrow_mut().take())))
        .build()
        .unwrap()
        .block_on(Box::pin(initial_reads(ExecutionKind::Operator, false)));
}
#[tokio::test]
async fn agent_cancel_before_initialization_returns_the_terminal_reason() {
    Box::pin(initial_reads(ExecutionKind::Agent, true)).await;
}
#[tokio::test]
async fn operator_cancel_before_initialization_returns_the_terminal_reason() {
    Box::pin(initial_reads(ExecutionKind::Operator, true)).await;
}

#[tokio::test]
async fn queued_continuations_keep_agent_and_operator_event_streams_open() {
    let _config = isolated_config();
    for kind in [ExecutionKind::Agent, ExecutionKind::Operator] {
        let root = tempfile::tempdir().unwrap();
        let client = Arc::new(InterruptClient {
            calls: Default::default(),
            first_release: Default::default(),
            resumed_release: Default::default(),
        });
        let node = worker_with_client(root.path(), client.clone()).await;
        let configured = node
            .handle(NodeOperation::Maintenance {
                command: ExecutionCommand {
                    action: "configure_scheduling".into(),
                    input: json!({"max_runs":1,"queue_order":"fifo"}),
                },
            })
            .await;
        assert_eq!(configured.status, 200);
        let id = format!("{}-continued", kind.prefix());
        let first = assignment(
            &node,
            &id,
            kind,
            json!({"prompt":"first","title":"continued"}),
            None,
        );
        assert_eq!(
            node.handle(NodeOperation::Create { assignment: first })
                .await
                .status,
            200
        );
        client.first_release.notify_one();
        assert_eq!(settled(&node, &id).await["execution"]["status"], "idle");
        let holder = assignment(
            &node,
            "agent-holder",
            ExecutionKind::Agent,
            json!({"prompt":"hold the only slot","title":"slot holder"}),
            None,
        );
        assert_eq!(
            node.handle(NodeOperation::Create { assignment: holder })
                .await
                .status,
            200
        );
        let accepted = command(
            &node,
            &id,
            kind,
            "prompt",
            json!({
                "input_id":"continuation", "prompt":"continue", "delivery":"queue",
            }),
        )
        .await;
        assert_eq!(accepted.status, 202, "{accepted:?}");
        assert_eq!(accepted.body["status"], "pending");
        let execution = ExecutionRef {
            id: id.clone(),
            kind,
        };
        let events = node
            .handle(NodeOperation::Events {
                execution: execution.clone(),
                after: 0,
            })
            .await;
        assert_eq!(events.status, 200, "{events:?}");
        assert_eq!(events.body["finished"], false, "{events:?}");
        let snapshot = command(&node, &id, kind, "http", json!({"method":"GET","tail":""})).await;
        assert_eq!(snapshot.body["draining"], true, "{snapshot:?}");
        client.resumed_release.notify_one();
        settled(&node, "agent-holder").await;
        assert_eq!(settled(&node, &id).await["execution"]["status"], "idle");
        assert_eq!(client.calls.load(Ordering::SeqCst), 3);
        let events = node
            .handle(NodeOperation::Events {
                execution,
                after: 0,
            })
            .await;
        assert_eq!(events.body["finished"], true, "{events:?}");
        node.shutdown().await.unwrap();
    }
}
