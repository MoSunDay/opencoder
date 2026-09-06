use crate::{journal::Record, Worker};
use anyhow::{bail, Context, Result};
use opencoder_core::{fleet::*, message::now_ms, Config, Role};
use opencoder_team::{fs_store, types::*, TeamDispatcher, TeamRunConfig};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio_util::sync::CancellationToken;

struct Dispatcher {
    worker: Worker,
    config: Config,
    members: HashMap<String, opencoder_core::fleet::TeamMember>,
    cancel: CancellationToken,
    coordinator: String,
    failures: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl TeamDispatcher for Dispatcher {
    async fn ask(&self, _topic: Option<&str>, member: &str, prompt: &str) -> Result<String> {
        let result = self.ask_member(member, prompt).await;
        if let Err(error) = &result {
            self.failures
                .lock()
                .unwrap()
                .push(format!("{member}: {error:#}"));
        }
        result
    }
}
impl Dispatcher {
    async fn ask_member(&self, member: &str, prompt: &str) -> Result<String> {
        if self.cancel.is_cancelled() {
            bail!("team cancelled");
        }
        let member = self
            .members
            .get(member)
            .context("team member not in pinned definition")?;
        let id = format!("member-{}", ulid::Ulid::new());
        super::agent::create_session(
            &self.worker,
            &id,
            &member.agent,
            None,
            Some(format!("{} / {}", self.coordinator, member.role)),
            now_ms(),
        )
        .await?;
        let mut session = opencoder_session::resume(
            self.worker.inner.state.store.clone(),
            &id,
            self.config.clone(),
            self.worker.client(&self.config)?,
            self.worker.inner.state.workdir.clone(),
        )
        .await?;
        session.cancel = Some(self.cancel.child_token());
        let (sink, flush) = opencoder_session::spawn_event_flusher(
            Some(self.worker.inner.state.store.clone()),
            id.clone(),
        );
        let outcome = opencoder_session::run(
            &mut session,
            format!("你的职责：{}\n\n{prompt}", member.role),
            move |event| {
                if let Err(error) = sink.push(&event) {
                    tracing::error!(%error,"team event persistence channel failed");
                }
            },
        )
        .await;
        flush.await?;
        outcome?;
        let messages = self.worker.inner.state.store.load_messages(&id).await?;
        let text = messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.text())
            .unwrap_or_default();
        if text.trim().is_empty() {
            bail!("team member produced no answer");
        }
        Ok(text)
    }
}

pub(super) async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    let a = &record.assignment;
    let id = &a.index.id;
    let definition = serde_json::from_value::<TeamDefinition>(
        a.definition.clone().context("team definition missing")?,
    )?;
    let legacy = worker.inner.journal.lock().await.uses_legacy(id);
    let root = if legacy {
        worker.inner.layout.legacy_team_dir(id)?
    } else {
        worker.inner.layout.team_state_dir(a.index.kind, id)?
    };
    let cfg = TeamRunConfig {
        team_root: root.clone(),
        max_turns: config.team_max_turns,
        max_sub_turns: config.team_max_sub_turns,
    };
    let members: Vec<MemberRef> = definition
        .members
        .iter()
        .map(|m| MemberRef {
            node_id: m.id.clone(),
            name: m.role.clone(),
        })
        .collect();
    let captain = members
        .iter()
        .find(|m| m.node_id == definition.captain)
        .context("captain unavailable")?
        .clone();
    if !resume || fs_store::load_topic(&root, &definition.name, id).is_err() {
        fs_store::save_team(
            &root,
            &TeamMeta {
                name: definition.name.clone(),
                captain: captain.clone(),
                members: members
                    .iter()
                    .map(|m| opencoder_team::types::TeamMember {
                        node_id: m.node_id.clone(),
                        name: m.name.clone(),
                        capabilities: vec![m.name.clone()],
                        profiled_at: None,
                    })
                    .collect(),
                created_at: a.index.created_at,
                updated_at: a.index.created_at,
            },
        )?;
        fs_store::save_topic(
            &root,
            &TopicMeta {
                topic_id: id.clone(),
                team_name: definition.name.clone(),
                title: a.request.input["title"]
                    .as_str()
                    .unwrap_or(&definition.name)
                    .into(),
                requirement: a.request.input["prompt"]
                    .as_str()
                    .or(a.request.input["requirement"].as_str())
                    .context("team requirement missing")?
                    .into(),
                status: TOPIC_EXECUTING.into(),
                finish_reason: None,
                created_at: a.index.created_at,
                finished_at: None,
                captain,
                members,
                turns: vec![],
                final_summary: None,
            },
        )?;
    }
    let failures = Arc::new(std::sync::Mutex::new(vec![]));
    let dispatcher = Arc::new(Dispatcher {
        worker: worker.clone(),
        config,
        members: definition
            .members
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect(),
        cancel: cancel.clone(),
        coordinator: id.clone(),
        failures: failures.clone(),
    });
    let token = opencoder_team::CancelToken::new();
    let forward = token.clone();
    let fwd = tokio::spawn(async move {
        cancel.cancelled().await;
        forward.cancel();
    });
    let result = opencoder_team::run_topic(
        worker.inner.state.store.clone(),
        dispatcher,
        &cfg,
        &definition.name,
        id,
        token,
    )
    .await;
    fwd.abort();
    let result = result?;
    if !failures.lock().unwrap().is_empty() {
        bail!(
            "team members failed: {}",
            failures.lock().unwrap().join("; ")
        );
    }
    let status = match result.finish_reason.as_deref() {
        Some(FINISH_COMPLETE) => ExecutionStatus::Done,
        Some(FINISH_CANCELLED) => ExecutionStatus::Cancelled,
        _ => ExecutionStatus::Error,
    };
    Ok((status, json!(result)))
}
