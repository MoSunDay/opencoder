//! One external turn. The surrounding session loop retains input admission.
use super::decode::{self, Decoder, Projection};
use crate::{SessionEvent, SessionState};
use anyhow::{bail, ensure, Context, Result};
use opencoder_core::{ContentBlock, Message, Role};

pub async fn run_turn(
    session: &mut SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<bool> {
    let offset = match &session.harness.last_input_id {
        Some(id) => {
            session
                .messages
                .iter()
                .position(|m| &m.id == id)
                .context("harness input checkpoint missing from transcript")?
                + 1
        }
        None => 0,
    };
    let users: Vec<_> = session.messages[offset..]
        .iter()
        .filter(|m| m.role == Role::User)
        .cloned()
        .collect();
    if users.is_empty() {
        ensure!(!session.harness.in_flight, "Codex turn was interrupted unexpectedly; submit a new instruction to resume explicitly");
        return Ok(false);
    }
    ensure!(
        !session.harness.in_flight || session.harness.thread_id.is_some(),
        "Codex submission state is unknown without a thread ID; start a new session"
    );
    let mut prompt = users
        .iter()
        .map(Message::text)
        .collect::<Vec<_>>()
        .join("\n\n");
    if session.harness.thread_id.is_none() && session.harness.fork_from.is_none() {
        prompt = format!("# Agent instructions\n{}\n\nUse your Codex tools to fulfill these instructions.\n\n# User requirement\n{prompt}", session.agent.prompt);
    }
    if let Some(skill) = session.skill_prompt_cloned() {
        prompt = format!("# Active skill\n{skill}\n\n{prompt}");
    }
    let images = materialize_images(session, &users)?;
    session.harness.in_flight = true;
    session.harness.last_input_id = users.last().map(|m| m.id.clone());
    super::super::save(session).await?;
    let mut decoder = Decoder::default();
    decoder.prefix = crate::runner::new_id();
    if session.harness.fork_from.is_none() {
        decoder.thread = session.harness.thread_id.clone();
    }
    let mut process = match super::process::spawn(session, prompt, &images) {
        Ok(p) => p,
        Err(e) => {
            session.harness.in_flight = false;
            super::super::save(session).await?;
            return Err(e);
        }
    };
    let hard = session.cancel.clone().unwrap_or_default();
    let steer = session
        .turn_cancel
        .as_ref()
        .and_then(|t| t.lock().ok().map(|t| t.clone()))
        .unwrap_or_default();
    let result: Result<bool> = async {
        loop {
            let line = tokio::select! {
                biased;
                _ = hard.cancelled() => return Ok(true),
                _ = steer.cancelled() => return Ok(true),
                line = tokio::time::timeout(session.config.stream_idle_timeout(), process.lines.next_line()) => line.context("Codex event stream idle timeout")??,
            };
            let Some(line) = line else { break; };
            ensure!(line.len() <= 16 * 1024 * 1024, "Codex event exceeds 16 MiB");
            let (next, projection) = decode::decode(decoder.clone(), &line)?;
            decoder = next;
            if decoder.thread != session.harness.thread_id {
                if let Some(id) = &decoder.thread {
                    session.harness.thread_id = Some(id.clone());
                    session.harness.fork_from = None;
                    super::super::save(session).await?;
                }
            }
            if decoder.completed {
                if let Some(message) = session.messages.iter_mut().rev().find(|m| m.role == Role::Assistant && m.id.starts_with(&decoder.prefix)) {
                    message.usage = decoder.usage.clone();
                    if let Some(store) = &session.store { store.set_message_usage(&session.id, &message.id, &message.usage).await?; }
                }
                session.last_usage.input_tokens = decoder.usage.input_tokens;
                session.last_usage.output_tokens = decoder.usage.output_tokens;
                session.last_usage.total_tokens = decoder.usage.total_tokens;
            }
            apply(session, projection, on_event).await?;
            if let Some(error) = &decoder.failed { bail!("Codex: {error}"); }
        }
        let (status, mut stderr) = tokio::time::timeout(session.config.stream_idle_timeout(), process.finish()).await.context("Codex exit timed out")??;
        for value in session.harness.envs.values().filter(|v| !v.is_empty()) { stderr = stderr.replace(value, "[redacted]"); }
        ensure!(status.success(), "Codex exited {status}: {stderr}");
        ensure!(decoder.completed && decoder.thread.is_some(), "Codex exited without a completed turn and thread ID: {stderr}");
        Ok(false)
    }.await;
    if !matches!(result, Ok(false)) {
        process.stop().await?;
        let reason = if result.is_ok() {
            "interrupted"
        } else {
            "Codex turn failed"
        };
        apply(session, decode::interrupt(&decoder, reason), on_event).await?;
        if decoder.started && !decoder.completed {
            on_event(SessionEvent::LlmRoundEnd);
        }
    }
    session.harness.in_flight = false;
    super::super::save(session).await?;
    if let Err(error) = &result {
        if decoder.failed.is_none() {
            on_event(SessionEvent::Error(format!("{error:#}")));
        }
    }
    result
}

async fn apply(
    session: &mut SessionState,
    projection: Projection,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    for mut message in projection.messages {
        message.agent = Some(session.agent.name.clone());
        message.model = session.harness.model.clone();
        session.record_checked(message).await?;
    }
    for event in projection.events {
        on_event(event);
    }
    Ok(())
}

fn materialize_images(
    session: &SessionState,
    messages: &[Message],
) -> Result<Vec<std::path::PathBuf>> {
    use base64::Engine;
    let mut paths = Vec::new();
    for m in messages {
        for block in &m.blocks {
            if let ContentBlock::Image { url, .. } = block {
                let (header, data) = url
                    .split_once(",")
                    .context("Codex images require an embedded image")?;
                ensure!(
                    header.starts_with("data:image/") && header.ends_with(";base64"),
                    "unsupported Codex image encoding"
                );
                let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
                let root = session
                    .harness
                    .resource_root
                    .as_ref()
                    .context("Codex workspace snapshot missing")?;
                let path = root.join(format!("image-{}.bin", ulid::Ulid::new()));
                std::fs::write(&path, bytes)?;
                paths.push(path);
            }
        }
    }
    Ok(paths)
}
