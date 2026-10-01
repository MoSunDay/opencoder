use super::client::ServerClient;
use crate::{chat::ChatView, session_ui::replay_messages};
use anyhow::{Context, Result};
use base64::Engine;
use opencoder_core::{
    fleet::{MessageCursor, MessagePage},
    Message, Role,
};
use opencoder_session::SessionEvent;
use serde_json::Value;

pub async fn messages(client: &ServerClient, id: &str) -> Result<Vec<Message>> {
    let mut cursor = MessageCursor::default();
    let mut result = Vec::new();
    let mut buffer = Vec::new();
    let mut sequence = None;
    loop {
        let page: MessagePage = serde_json::from_value(client.command(id, "http", serde_json::json!({
            "method":"GET", "tail": format!("transcript?seq={}&offset={}",cursor.seq,cursor.offset)
        })).await?)?;
        for chunk in page.chunks {
            anyhow::ensure!(chunk.encoding == "base64", "unsupported message encoding");
            anyhow::ensure!(
                chunk.offset == buffer.len() as u64 && sequence.is_none_or(|seq| seq == chunk.seq),
                "invalid message chunk cursor"
            );
            sequence = Some(chunk.seq);
            buffer.extend(base64::engine::general_purpose::STANDARD.decode(chunk.bytes_b64)?);
            anyhow::ensure!(
                chunk.next_offset == buffer.len() as u64,
                "invalid message chunk length"
            );
            if chunk.eof {
                anyhow::ensure!(
                    buffer.len() as u64 == chunk.total_bytes,
                    "incomplete message"
                );
                let message: Message =
                    serde_json::from_slice(&buffer).context("decode remote transcript")?;
                result.push(message);
                sequence = None;
                buffer.clear();
            }
        }
        match page.next_cursor {
            Some(next) => {
                anyhow::ensure!(next != cursor, "Server message cursor did not advance");
                cursor = next;
            }
            None => break,
        }
    }
    anyhow::ensure!(buffer.is_empty(), "incomplete remote transcript");
    Ok(result)
}

pub async fn events(client: &ServerClient, id: &str) -> Result<(Vec<Value>, i64)> {
    let mut result = Vec::new();
    let mut cursor = 0;
    loop {
        let page: Value = client
            .get(&format!("/api/executions/{id}/events-page?after={cursor}"))
            .await?;
        let frames = page["events"]
            .as_array()
            .context("invalid Server event page")?;
        for frame in frames {
            let seq = frame["seq"].as_i64().context("missing event cursor")?;
            anyhow::ensure!(seq > cursor, "Server event cursor did not advance");
            cursor = seq;
            let mut frame = frame.clone();
            frame["data"] = event_data(client, id, Some(seq), frame["data"].clone()).await?;
            result.push(frame);
        }
        if page["more"] != true {
            break;
        }
        anyhow::ensure!(!frames.is_empty(), "empty Server event page with more=true");
    }
    Ok((result, cursor))
}

/// Replay durable events using the same renderer as live native/Codex turns.
/// Queue/steer events define the snapshot boundary. Stored messages supply
/// images and provide boundaries for container runners without echo events.
pub fn replay(label: &str, messages: &[Message], events: &[Value]) -> ChatView {
    if events.is_empty()
        || events
            .iter()
            .any(|event| event["kind"] == "transcript_reset")
    {
        let mut chat = replay_messages(label, messages);
        chat.remote = true;
        chat.submitted = !messages.is_empty();
        replay_pending_round(&mut chat, messages, events);
        return chat;
    }
    let users: Vec<_> = messages
        .iter()
        .filter(|msg| msg.role == Role::User && (!msg.synthetic || msg.display.is_some()))
        .collect();
    let event_echoes = events.iter().any(|frame| {
        matches!(
            frame["kind"].as_str(),
            Some("queue_consumed" | "steer_consumed")
        )
    });
    let mut user = 0;
    let mut chat = ChatView {
        agent: label.into(),
        remote: true,
        ..Default::default()
    };
    for frame in events {
        let ts = frame["ts"].as_i64().unwrap_or(i64::MAX);
        while !event_echoes && user < users.len() && users[user].created_at <= ts {
            let echo = replay_messages(label, std::slice::from_ref(users[user]));
            chat.blocks.extend(echo.blocks);
            if chat.first_prompt.is_none() {
                chat.first_prompt = echo.first_prompt;
            }
            chat.begin_turn();
            user += 1;
        }
        if let Some(event) =
            SessionEvent::from_sse(frame["kind"].as_str().unwrap_or(""), frame["data"].clone())
        {
            match &event {
                SessionEvent::QueueConsumed { text, .. }
                | SessionEvent::SteerConsumed { text, .. } => {
                    if !text.is_empty() {
                        let stored = users[user..]
                            .iter()
                            .position(|message| {
                                message.display.as_deref().unwrap_or("") == text
                                    || message.text() == *text
                            })
                            .map(|index| user + index);
                        let echo = if let Some(index) = stored {
                            user = index + 1;
                            replay_messages(label, std::slice::from_ref(users[index]))
                        } else {
                            replay_messages(label, &[Message::user("echo", text)])
                        };
                        chat.blocks.extend(echo.blocks);
                        if chat.first_prompt.is_none() {
                            chat.first_prompt = echo.first_prompt;
                        }
                        chat.begin_turn();
                    }
                }
                SessionEvent::AgentSwitch(_) => {}
                _ => chat.apply(&event),
            }
        }
    }
    // Do not echo messages written after the last fetched event: their
    // consumption will arrive on the subscription after this cursor.
    chat.submitted = chat.first_prompt.is_some() || !chat.blocks.is_empty();
    chat
}

/// A compacted transcript has no durable assistant message for the current
/// streaming round. Preserve those deltas when resuming during execution.
fn replay_pending_round(chat: &mut ChatView, messages: &[Message], events: &[Value]) {
    let Some(start) = events
        .iter()
        .rposition(|event| event["kind"] == "llm_round_start")
    else {
        return;
    };
    if events[start..].iter().any(|event| {
        matches!(
            event["kind"].as_str(),
            Some("llm_round_end" | "done" | "error")
        )
    }) {
        return;
    }
    let started_at = events[start]["data"]["started_at_ms"]
        .as_i64()
        .unwrap_or(i64::MAX);
    if messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .is_some_and(|message| message.created_at >= started_at)
    {
        return;
    }
    for frame in &events[start..] {
        if matches!(
            frame["kind"].as_str(),
            Some("llm_round_start" | "reasoning_delta" | "text_delta" | "llm_attempt_reset")
        ) {
            if let Some(event) =
                SessionEvent::from_sse(frame["kind"].as_str().unwrap(), frame["data"].clone())
            {
                chat.apply(&event);
            }
        }
    }
}

/// Large tool outputs are paged by Server; fetch the referenced JSON payload
/// before decoding it as a SessionEvent.
pub async fn event_data(
    client: &ServerClient,
    id: &str,
    seq: Option<i64>,
    data: Value,
) -> Result<Value> {
    if data["omitted"] != true {
        return Ok(data);
    }
    let seq = seq.context("large event has no cursor")?;
    let mut bytes = Vec::new();
    loop {
        let chunk: opencoder_core::fleet::EventPayloadChunk = client
            .get(&format!(
                "/api/executions/{id}/events/{seq}/payload?offset={}",
                bytes.len()
            ))
            .await?;
        anyhow::ensure!(
            chunk.seq == seq && chunk.offset == bytes.len() as u64,
            "invalid event payload cursor"
        );
        bytes.extend(base64::engine::general_purpose::STANDARD.decode(chunk.bytes_b64)?);
        anyhow::ensure!(
            chunk.next_offset == bytes.len() as u64 && chunk.next_offset > chunk.offset,
            "event payload did not advance"
        );
        if chunk.eof {
            anyhow::ensure!(
                bytes.len() as u64 == chunk.total_bytes,
                "incomplete event payload"
            );
            return Ok(serde_json::from_slice(&bytes)?);
        }
    }
}
