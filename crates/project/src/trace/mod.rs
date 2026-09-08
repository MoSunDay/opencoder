//! Per-attempt input, immutable model/event files and session boundaries.
pub mod archive;
mod artifacts;
mod children;
mod client;
pub(crate) mod codex;
pub(crate) mod recovery;
pub mod resources;
use crate::service::Deps;
use anyhow::{Context, Result};
use archive::Archive;
use opencoder_store::{ProjectTodoRunPatch, ProjectTodoRunRecord};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

pub struct RunTrace {
    pub archive: Archive,
    run_id: String,
    session_id: String,
    working_dir: PathBuf,
    deliverable_manifest: Option<PathBuf>,
    message_start: i64,
    event_start: i64,
    children: Vec<String>,
    started_children: std::sync::Mutex<Vec<(String, String)>>,
}

pub fn root(deps: &Deps) -> PathBuf {
    deps.archive_root.lock().unwrap().clone()
}
pub fn input(run: &ProjectTodoRunRecord) -> Result<Value> {
    serde_json::from_str(
        run.input_snapshot
            .as_deref()
            .context("historical run has incomplete input retention")?,
    )
    .context("invalid project input snapshot")
}
pub fn manifest(run: &ProjectTodoRunRecord) -> Result<Value> {
    serde_json::from_str(
        run.trace_manifest
            .as_deref()
            .context("historical run has incomplete trace retention")?,
    )
    .context("invalid project trace manifest")
}

impl RunTrace {
    pub async fn begin(
        deps: &Deps,
        run_id: &str,
        session: &mut opencoder_session::SessionState,
        prompt: &str,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let rec = deps
            .projects
            .get_todo_run(run_id)
            .await?
            .context("project run missing")?;
        let mut input = input(&rec)?;
        let message_start = deps.store.last_message_seq(&session.id).await?;
        let event_start = deps.store.last_event_seq(&session.id).await?;
        let children = deps
            .store
            .list_subagent_tasks(&session.id)
            .await?
            .into_iter()
            .map(|t| t.task_id)
            .collect();
        let archive = Archive::create(archive::run_root(&root(deps), run_id)?, cancel)?;
        input["prompt"] = json!(prompt);
        input["agent"] = resources::identity(&session.agent)?;
        input["harness"] = json!(session.harness.harness);
        if session.harness.harness == opencoder_core::harness::Harness::Codex {
            input["model"] = json!(session.harness.model);
            input["model_source"] = json!(if session.harness.model.is_some() {
                "launch"
            } else {
                "codex_config"
            });
            input["reasoning_effort"] = Value::Null;
        } else {
            input["model"] = json!(session.config.model);
            input["reasoning_effort"] = json!(session.config.reasoning_effort);
        }
        archive::write_new(&archive.root.join("input.json"), &input)?;
        let trace = json!({"schema":1,"complete":false,"session_id":session.id,"messages_after":message_start,"events_after":event_start,"children_before":children});
        anyhow::ensure!(
            deps.projects
                .patch_todo_run(
                    run_id,
                    &ProjectTodoRunPatch {
                        input_snapshot: Some(input.to_string()),
                        trace_manifest: Some(trace.to_string()),
                        session_id: Some(session.id.clone()),
                        ..Default::default()
                    },
                    opencoder_core::message::now_ms()
                )
                .await?,
            "project run disappeared before session start"
        );
        session.client = Arc::new(client::RecordedClient {
            inner: session.client.clone(),
            archive: archive.clone(),
        });
        Ok(Self {
            archive,
            run_id: run_id.into(),
            session_id: session.id.clone(),
            working_dir: session.working_dir.clone(),
            deliverable_manifest: codex::manifest_path(session, run_id),
            message_start,
            event_start,
            children,
            started_children: std::sync::Mutex::new(Vec::new()),
        })
    }
    pub fn tools(&self) -> opencoder_session::extensions::Registration {
        artifacts::install(&self.session_id, self.archive.clone())
    }
    pub fn record(&self, event: &opencoder_session::SessionEvent) {
        children::started(event, &mut self.started_children.lock().unwrap());
        if let Err(error) = self.archive.event(event.sse_kind(), event.sse_data()) {
            self.archive.fail(error);
        }
    }
    pub fn collect_deliverables(&self) -> Result<()> {
        codex::collect(
            &self.archive,
            &self.working_dir,
            self.deliverable_manifest.as_deref(),
        )
    }
    pub async fn finish(&self, deps: &Deps) -> Result<()> {
        self.archive.check()?;
        let end = deps.store.last_message_seq(&self.session_id).await?;
        let event_end = deps.store.last_event_seq(&self.session_id).await?;
        let expected = self.started_children.lock().unwrap().clone();
        let children = children::links(deps, &self.session_id, &self.children, &expected).await?;
        let (events, calls) = self.archive.counts();
        let files = std::fs::read_dir(&self.archive.root)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<Vec<_>>>()?;
        let mut artifacts = Vec::new();
        for name in files
            .iter()
            .filter(|n| n.starts_with("artifact-") && n.ends_with(".json"))
        {
            artifacts.push(serde_json::from_slice::<Value>(&std::fs::read(
                self.archive.root.join(name),
            )?)?);
        }
        let mut trace = json!({"schema":1,"complete":true,"session_id":self.session_id,"messages_after":self.message_start,"messages_through":end,"events_after":self.event_start,"events_through":event_end,"event_count":events,"model_calls":calls,"children":children,"artifacts":artifacts,"files":files.into_iter().filter(|name|name.starts_with("request-")||name.starts_with("response-")).collect::<Vec<_>>()});
        if let Some(runtime) = deps.store.harness_runtime(&self.session_id).await? {
            trace["harness"] = json!(runtime.harness);
            trace["thread_id"] = json!(runtime.thread_id);
        }
        archive::write_new(&self.archive.root.join("manifest.json"), &trace)?;
        anyhow::ensure!(
            deps.projects
                .patch_todo_run(
                    &self.run_id,
                    &ProjectTodoRunPatch {
                        trace_manifest: Some(trace.to_string()),
                        ..Default::default()
                    },
                    opencoder_core::message::now_ms()
                )
                .await?,
            "project run disappeared before trace completion"
        );
        Ok(())
    }
}
