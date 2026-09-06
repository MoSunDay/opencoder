use crate::{journal::Record, lifecycle::Lifecycle, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, message::now_ms};
use serde_json::{json, Value};
pub(super) async fn fork(worker: &Worker, parent: &str) -> Result<RpcReply> {
    let id = format!("agent-{}", ulid::Ulid::new());
    let (legacy, parent_kind) = {
        let journal = worker.inner.journal.lock().await;
        let kind = journal
            .records
            .get(parent)
            .map(|record| record.assignment.index.kind)
            .unwrap_or(ExecutionKind::Agent);
        (journal.uses_legacy(parent), kind)
    };
    let source = if legacy {
        worker.inner.layout.legacy_resources_dir(parent)?
    } else {
        worker.inner.layout.resources_dir(parent_kind, parent)?
    };
    if source.exists() {
        crate::resources::pin(
            Some(&source),
            &worker
                .inner
                .layout
                .resources_dir(ExecutionKind::Agent, &id)?,
        )?;
    }
    opencoder_session::fork::fork_session_with_id(worker.inner.state.store.as_ref(), parent, &id)
        .await?;
    let meta = worker.inner.state.store.get_session(&id).await?.unwrap();
    let index = ExecutionIndex {
        id: id.clone(),
        created_at: now_ms(),
        kind: ExecutionKind::Agent,
        node_id: worker.inner.registration.id.clone(),
        status: ExecutionStatus::Idle,
    };
    let request = CreateExecution {
        id: id.clone(),
        kind: ExecutionKind::Agent,
        target: meta.agent,
        input: json!({"forked_from":parent}),
        node_id: Some(index.node_id.clone()),
    };
    worker.inner.journal.lock().await.save(Record {
        assignment: Assignment {
            index,
            request,
            definition: None,
        },
        result: Value::Null,
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    })?;
    opencoder_session::loop_registry::notify_change();
    Ok(RpcReply::ok(json!({"id":id})))
}
