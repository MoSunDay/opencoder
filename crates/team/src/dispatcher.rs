//! Prompt fan-out to team nodes. The trait is the seam: the real
//! `NodeDispatcher` drives the store's node-task machinery, tests script a
//! `MockDispatcher`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use opencoder_core::{message::now_ms, Role};
use opencoder_store::{NodeTaskStatus, Store, TeamTopicRunRecord, TEAM_RUN_EXECUTING};
use ulid::Ulid;

/// Dispatch one prompt to one node and return the task's final assistant
/// text. `topic == None` means "not part of a topic run" (e.g. capability
/// profiling) and writes no `team_topic_runs` row.
#[async_trait]
pub trait TeamDispatcher: Send + Sync {
    async fn ask(&self, topic: Option<&str>, node_id: &str, prompt: &str) -> Result<String>;
}

pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
pub const DEFAULT_TASK_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Real dispatcher: one synthetic node task per ask (agent=None), a
/// `team_topic_runs` ledger row per (topic, node), then poll to terminal.
pub struct NodeDispatcher {
    pub store: Arc<dyn Store>,
    pub poll_interval: Duration,
    pub task_timeout: Duration,
}

impl NodeDispatcher {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            poll_interval: DEFAULT_POLL_INTERVAL,
            task_timeout: DEFAULT_TASK_TIMEOUT,
        }
    }

    pub fn with_timeouts(
        store: Arc<dyn Store>,
        poll_interval: Duration,
        task_timeout: Duration,
    ) -> Self {
        Self {
            store,
            poll_interval,
            task_timeout,
        }
    }
}

#[async_trait]
impl TeamDispatcher for NodeDispatcher {
    async fn ask(&self, topic: Option<&str>, node_id: &str, prompt: &str) -> Result<String> {
        let task_id = Ulid::new().to_string();
        let session_id = Ulid::new().to_string();
        let now = now_ms();
        let title = topic.unwrap_or("team").to_string();
        self.store
            .dispatch_node_task(
                &task_id,
                &session_id,
                node_id,
                Some(&title),
                prompt,
                None,
                None,
                now,
            )
            .await?;
        if let Some(topic_id) = topic {
            self.store
                .upsert_team_topic_run(&TeamTopicRunRecord {
                    topic_id: topic_id.to_string(),
                    node_id: node_id.to_string(),
                    status: TEAM_RUN_EXECUTING.to_string(),
                    created_at: now,
                })
                .await?;
        }
        let deadline = tokio::time::Instant::now() + self.task_timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                let _ = self.store.request_node_task_cancel(&task_id).await;
                return Err(anyhow!(
                    "node task {task_id} on {node_id} timed out after {:?}",
                    self.task_timeout
                ));
            }
            tokio::time::sleep(self.poll_interval).await;
            let Some(task) = self.store.get_node_task(&task_id).await? else {
                continue;
            };
            match task.status {
                NodeTaskStatus::Done => {
                    let messages = self.store.load_messages(&task.session_id).await?;
                    let text = messages
                        .iter()
                        .rev()
                        .find(|m| m.role == Role::Assistant)
                        .map(|m| m.text())
                        .unwrap_or_default();
                    if text.trim().is_empty() {
                        return Err(anyhow!(
                            "node task {task_id} finished with no assistant text"
                        ));
                    }
                    return Ok(text);
                }
                NodeTaskStatus::Error => {
                    return Err(anyhow!(
                        "node task {task_id} failed: {}",
                        task.error.unwrap_or_else(|| "unknown error".into())
                    ));
                }
                NodeTaskStatus::Cancelled => {
                    return Err(anyhow!("node task {task_id} was cancelled"));
                }
                _ => {}
            }
        }
    }
}

// ── test double ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DispatchCall {
    pub topic: Option<String>,
    pub node_id: String,
    pub prompt: String,
}

/// Scripted dispatcher: per-node FIFO reply queues (parallel-safe — a node's
/// sequence never depends on interleaving with other nodes) plus a full call
/// log for assertions. Optionally mirrors the real dispatcher's
/// `team_topic_runs` upserts when a store is attached.
#[derive(Default)]
pub struct MockDispatcher {
    store: Option<Arc<dyn Store>>,
    replies: Mutex<HashMap<String, VecDeque<Result<String, String>>>>,
    calls: Mutex<Vec<DispatchCall>>,
}

/// Script helpers: `mock.reply("n1", vec![ok(json), err("boom")])`.
pub fn ok(text: impl Into<String>) -> Result<String, String> {
    Ok(text.into())
}

pub fn err(text: impl Into<String>) -> Result<String, String> {
    Err(text.into())
}

impl MockDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_store(store: Arc<dyn Store>) -> Self {
        Self {
            store: Some(store),
            ..Self::default()
        }
    }

    /// Builder: append a node's replies (later `.reply` calls on the same
    /// node queue after earlier ones).
    pub fn reply(self, node_id: &str, replies: Vec<Result<String, String>>) -> Self {
        self.replies
            .lock()
            .expect("mock replies lock")
            .entry(node_id.to_string())
            .or_default()
            .extend(replies);
        self
    }

    pub fn calls(&self) -> Vec<DispatchCall> {
        self.calls.lock().expect("mock calls lock").clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("mock calls lock").len()
    }

    pub fn calls_for(&self, node_id: &str) -> Vec<DispatchCall> {
        self.calls
            .lock()
            .expect("mock calls lock")
            .iter()
            .filter(|c| c.node_id == node_id)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl TeamDispatcher for MockDispatcher {
    async fn ask(&self, topic: Option<&str>, node_id: &str, prompt: &str) -> Result<String> {
        self.calls
            .lock()
            .expect("mock calls lock")
            .push(DispatchCall {
                topic: topic.map(str::to_string),
                node_id: node_id.to_string(),
                prompt: prompt.to_string(),
            });
        if let (Some(store), Some(topic_id)) = (&self.store, topic) {
            store
                .upsert_team_topic_run(&TeamTopicRunRecord {
                    topic_id: topic_id.to_string(),
                    node_id: node_id.to_string(),
                    status: TEAM_RUN_EXECUTING.to_string(),
                    created_at: now_ms(),
                })
                .await?;
        }
        let reply = self
            .replies
            .lock()
            .expect("mock replies lock")
            .get_mut(node_id)
            .and_then(|queue| queue.pop_front())
            .ok_or_else(|| anyhow!("mock dispatcher: no scripted reply left for node {node_id}"))?;
        reply.map_err(|message| anyhow!("mock dispatcher error for {node_id}: {message}"))
    }
}
