use super::artifacts::Artifact;
use anyhow::{ensure, Result};
use opencoder_core::{message::now_ms, ContentBlock, Message, Role};
use opencoder_session::{
    harness::codex::decode::{self, Decoder},
    SessionEvent, SessionState,
};
use opencoder_store::{EventKind, SessionEventRecord};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Stage {
        stage: String,
        #[serde(default)]
        detail: Value,
    },
    Codex {
        phase: String,
        event: Value,
    },
    Result {
        result_file: String,
        artifacts: Vec<Artifact>,
    },
    Error {
        error: String,
    },
}

pub struct Events {
    pub status_path: std::path::PathBuf,
    pub decoders: BTreeMap<String, Decoder>,
    pub result: Option<(String, Vec<Artifact>)>,
    pub prefix: String,
    pub step: String,
}

impl Events {
    pub async fn accept(&mut self, session: &mut SessionState, value: Event) -> Result<()> {
        ensure!(
            self.result.is_none(),
            "Runner emitted events after its result"
        );
        match value {
            Event::Stage { stage, detail } => {
                ensure!(
                    !stage.is_empty() && stage.len() <= 128,
                    "invalid Runner stage"
                );
                crate::checkpoint::write(
                    &self.status_path,
                    &serde_json::to_vec(&json!({"stage":stage,"detail":detail}))?,
                )?;
                session
                    .store
                    .as_ref()
                    .unwrap()
                    .append_events(&[SessionEventRecord {
                        session_id: session.id.clone(),
                        kind: EventKind::Step,
                        ts: now_ms(),
                        seq: None,
                        sse_kind: Some("runner_stage".into()),
                        payload: json!({"step":self.step,"stage":stage,"detail":detail}),
                    }])
                    .await?;
                session
                    .record_checked(Message {
                        provider_state: None,
                        id: format!("{}-stage-{}", self.prefix, ulid::Ulid::new()),
                        role: Role::User,
                        blocks: vec![ContentBlock::text(format!("执行阶段：{stage}"))],
                        model: session.harness.model.clone(),
                        agent: Some(session.agent.name.clone()),
                        created_at: now_ms(),
                        usage: Default::default(),
                        synthetic: true,
                        display: None,
                    })
                    .await?;
            }
            Event::Codex { phase, event } => {
                ensure!(
                    !phase.is_empty() && phase.len() <= 64,
                    "invalid Runner Codex phase"
                );
                let previous = self.decoders.get(&phase).cloned().unwrap_or_else(|| {
                    let mut decoder = Decoder::default();
                    decoder.prefix = format!("{}:{phase}", self.prefix);
                    decoder
                });
                let (next, effects) = decode::decode(previous, &serde_json::to_string(&event)?)?;
                for mut message in effects.messages {
                    message.agent = Some(session.agent.name.clone());
                    message.model = session.harness.model.clone();
                    session.record_checked(message).await?;
                }
                let rows: Vec<_> = effects
                    .events
                    .iter()
                    .map(|event| record(session, event))
                    .collect();
                session.store.as_ref().unwrap().append_events(&rows).await?;
                if let Some(error) = &next.failed {
                    anyhow::bail!("Codex {phase}: {error}");
                }
                if next.completed {
                    if let Some(message) = session
                        .messages
                        .iter_mut()
                        .rev()
                        .find(|m| m.role == Role::Assistant && m.id.starts_with(&next.prefix))
                    {
                        message.usage = next.usage.clone();
                        session
                            .store
                            .as_ref()
                            .unwrap()
                            .set_message_usage(&session.id, &message.id, &message.usage)
                            .await?;
                    }
                }
                self.decoders.insert(phase, next);
            }
            Event::Result {
                result_file,
                artifacts,
            } => {
                ensure!(
                    !self.decoders.is_empty()
                        && self
                            .decoders
                            .values()
                            .all(|d| d.completed && d.failed.is_none() && d.thread.is_some()),
                    "Runner completed with unfinished Codex phases"
                );
                self.result = Some((result_file, artifacts));
            }
            Event::Error { error } => anyhow::bail!("Runner: {error}"),
        }
        Ok(())
    }

    pub async fn interrupt(&self, session: &mut SessionState, reason: &str) -> Result<()> {
        for decoder in self.decoders.values().filter(|d| !d.completed) {
            let effects = decode::interrupt(decoder, reason);
            for message in effects.messages {
                session.record_checked(message).await?;
            }
            let rows: Vec<_> = effects
                .events
                .iter()
                .map(|event| record(session, event))
                .collect();
            session.store.as_ref().unwrap().append_events(&rows).await?;
        }
        Ok(())
    }
}

fn record(session: &SessionState, event: &SessionEvent) -> SessionEventRecord {
    SessionEventRecord {
        session_id: session.id.clone(),
        kind: event.coarse_kind(),
        payload: event.sse_data(),
        ts: now_ms(),
        seq: None,
        sse_kind: Some(event.sse_kind().into()),
    }
}
