//! Real `NodeDispatcher` against a real LibsqlStore: the done path (claim →
//! answer → complete → last-assistant-text) and the timeout path (cancel
//! request + error), plus the `team_topic_runs` ledger upsert.

mod common;

use std::time::Duration;

use common::*;
use opencoder_core::{message::now_ms, ContentBlock, Message};
use opencoder_store::{NodeTaskStatus, Store, TEAM_RUN_EXECUTING};
use opencoder_team::{NodeDispatcher, TeamDispatcher};
use ulid::Ulid;

fn assistant_text(text: &str) -> Message {
    let mut message = Message::assistant(Ulid::new().to_string());
    message.blocks.push(ContentBlock::text(text));
    message
}

#[tokio::test]
async fn ask_returns_last_assistant_text_and_upserts_ledger() {
    let fx = fixture(2, 1).await;
    let node = register(&fx.store, "worker").await;
    let dispatcher = fast_dispatcher(fx.store.clone());
    let topic = Ulid::new().to_string();

    let ask = {
        let dispatcher = std::sync::Arc::new(dispatcher);
        let topic = topic.clone();
        let node_id = node.id.clone();
        tokio::spawn(async move { dispatcher.ask(Some(&topic), &node_id, "请回答").await })
    };

    // Simulate the worker claiming and completing its task.
    let task = loop {
        if let Some(task) = fx
            .store
            .claim_next_node_task(&node.id, now_ms())
            .await
            .unwrap()
        {
            break task;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    fx.store
        .append_message(&task.session_id, &assistant_text("来自节点的回答"))
        .await
        .unwrap();
    fx.store
        .update_node_task_status(&task.id, NodeTaskStatus::Done, None, now_ms())
        .await
        .unwrap();

    let text = ask.await.unwrap().unwrap();
    assert_eq!(text, "来自节点的回答");
    let rows = fx.store.list_team_topic_runs(&topic).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].topic_id, topic);
    assert_eq!(rows[0].node_id, node.id);
    assert_eq!(rows[0].status, TEAM_RUN_EXECUTING);
}

#[tokio::test]
async fn ask_error_task_surfaces_node_error() {
    let fx = fixture(2, 1).await;
    let node = register(&fx.store, "worker").await;
    let dispatcher = std::sync::Arc::new(fast_dispatcher(fx.store.clone()));
    let node_id = node.id.clone();
    let ask = tokio::spawn(async move { dispatcher.ask(None, &node_id, "请回答").await });

    let task = loop {
        if let Some(task) = fx
            .store
            .claim_next_node_task(&node.id, now_ms())
            .await
            .unwrap()
        {
            break task;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    fx.store
        .update_node_task_status(&task.id, NodeTaskStatus::Error, Some("模型超时"), now_ms())
        .await
        .unwrap();
    let error = ask.await.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("模型超时"), "{error:#}");
}

#[tokio::test]
async fn ask_timeout_requests_cancel() {
    let fx = fixture(2, 1).await;
    let node = register(&fx.store, "worker").await;
    // Nobody ever claims the task: the dispatcher must time out, request the
    // cancel, and surface an error.
    let dispatcher = NodeDispatcher::with_timeouts(
        fx.store.clone(),
        Duration::from_millis(10),
        Duration::from_millis(80),
    );
    let error = dispatcher
        .ask(None, &node.id, "没人理我")
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("timed out"), "{error:#}");
    let task = fx
        .store
        .list_node_tasks_filtered(Some(&node.id), None, 10)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("task row exists");
    assert!(
        task.cancel_requested || task.status.is_terminal(),
        "cancel was requested"
    );
}
