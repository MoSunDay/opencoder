//! Unit tests for the DAG single-step event query
//! (`operations::query::dag_step_events`): event source selection, step
//! filtering, paging/cursor behaviour and the terminal `step_finished` frame.

use super::*;
use crate::Worker;
use opencoder_core::fleet::{ExecutionKind, ExecutionRef, ExecutionStatus, RpcReply};
use opencoder_store::SessionMeta;
use serde_json::{json, Value};

/// A journaled DAG run whose definition snapshot carries the given steps.
async fn dag_run(worker: &Worker, id: &str, steps: Vec<Value>) {
    let mut accepted = record(worker, id, ExecutionStatus::Running);
    accepted.assignment.index.kind = ExecutionKind::Dag;
    accepted.assignment.request.kind = ExecutionKind::Dag;
    accepted.assignment.definition = Some(json!({"spec": {"name": id, "steps": steps}}));
    worker.inner.journal.lock().await.save(accepted).unwrap();
}

fn step_spec(name: &str, kind: &str) -> Value {
    let spec = json!({"name": name});
    let mut spec = spec.as_object().unwrap().clone();
    spec.insert(
        "kind".into(),
        match kind {
            "wasm" => json!({"type": "wasm", "command": "tool.wasm"}),
            "runner" => json!({"type": "runner", "command": "codex exec"}),
            _ => json!({"type": "agent", "prompt": "go"}),
        },
    );
    Value::Object(spec)
}

fn dag_ref(id: &str) -> ExecutionRef {
    ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Dag,
    }
}

async fn session(worker: &Worker, id: &str) {
    worker
        .inner
        .state
        .store
        .create_session(&SessionMeta {
            id: id.into(),
            created_at: 1,
            updated_at: 1,
            ..SessionMeta::default()
        })
        .await
        .unwrap();
}

/// Append `(sse_kind, payload)` rows; returns the assigned seqs.
async fn append(worker: &Worker, id: &str, rows: Vec<(&str, Value)>) -> Vec<i64> {
    let entries: Vec<_> = rows
        .into_iter()
        .enumerate()
        .map(
            |(index, (kind, payload))| opencoder_store::SessionEventRecord {
                session_id: id.into(),
                kind: opencoder_store::EventKind::Step,
                payload,
                ts: index as i64 + 1,
                seq: None,
                sse_kind: Some(kind.into()),
            },
        )
        .collect();
    worker
        .inner
        .state
        .store
        .append_events(&entries)
        .await
        .unwrap()
}

/// Write step artifacts under the node's DAG root (`<data>/dag/<run>/<step>`).
fn artifacts(worker: &Worker, run: &str, step: &str, files: &[(&str, Value)]) {
    let root = worker.inner.layout.kind_root(ExecutionKind::Dag);
    let dir = opencoder_dag::artifacts::step_dir(&root, run, step).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    for (name, value) in files {
        std::fs::write(dir.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }
}

fn frames(reply: &RpcReply) -> &Vec<Value> {
    reply.body["events"].as_array().unwrap()
}

mod paging;
mod sources;
