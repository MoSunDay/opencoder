use crate::{journal::Record, lifecycle::Lifecycle, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, message::now_ms};
use serde_json::{json, Value};
pub(super) async fn fork(worker: &Worker, parent: &str) -> Result<RpcReply> {
    let (legacy, parent_kind, codex, runtime, environment_version) = {
        let journal = worker.inner.journal.lock().await;
        let record = journal.get(parent).await?;
        (
            journal.uses_legacy(parent),
            record
                .as_ref()
                .map(|r| r.assignment.index.kind)
                .unwrap_or(ExecutionKind::Agent),
            record.as_ref().and_then(|r| r.assignment.codex.clone()),
            record.as_ref().and_then(|r| r.assignment.runtime.clone()),
            record
                .as_ref()
                .and_then(|r| r.annotations.get("operator_environment_version").cloned()),
        )
    };
    anyhow::ensure!(
        matches!(parent_kind, ExecutionKind::Agent | ExecutionKind::Operator),
        "only conversations can be forked"
    );
    let id = format!("{}-{}", parent_kind.prefix(), ulid::Ulid::new());
    let config = worker.configuration_for(parent_kind)?;
    let source = if legacy {
        worker.inner.layout.legacy_resources_dir(parent)?
    } else {
        worker.inner.layout.resources_dir(parent_kind, parent)?
    };
    if source.exists() {
        crate::resources::pin(
            Some(&source),
            &worker.inner.layout.resources_dir(parent_kind, &id)?,
        )?;
    }
    let environment = super::operator_env::fork_environment(
        &worker.inner.layout,
        parent_kind,
        parent,
        &id,
        environment_version.as_ref(),
        &config,
    )?;
    opencoder_session::fork::fork_session_in(
        worker.inner.state.store.as_ref(),
        parent,
        &id,
        environment.as_ref().map(|env| env.workspace.as_path()),
    )
    .await?;
    if let Some(env) = &environment {
        if let Some(mut harness) = worker.inner.state.store.harness_runtime(&id).await? {
            harness.envs.extend(super::operator_env::env_pairs(
                &env.home,
                config.operator.access,
            ));
            worker
                .inner
                .state
                .store
                .set_harness_runtime(&id, &harness)
                .await?;
        }
    }
    let meta = worker.inner.state.store.get_session(&id).await?.unwrap();
    let index = ExecutionIndex {
        id: id.clone(),
        created_at: now_ms(),
        kind: parent_kind,
        node_id: worker.inner.registration.id.clone(),
        status: ExecutionStatus::Idle,
    };
    let request = CreateExecution {
        id: id.clone(),
        kind: parent_kind,
        target: meta.agent,
        input: json!({"forked_from":parent}),
        node_id: Some(index.node_id.clone()),
    };
    worker.inner.journal.lock().await.save(Record {
        annotations: if environment.is_some() {
            json!({"operator_environment_version": 1,
                "operator_identity": super::operator_env::identity_annotation(&environment)})
        } else {
            Value::Null
        },
        queue: None,
        assignment: Assignment {
            private_context: None,
            runtime,
            codex,
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
