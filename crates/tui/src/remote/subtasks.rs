//! One-shot remote subtask dispatch and server-side projections.
//!
//! A subtask is a fresh server execution created with a single question and
//! never driven interactively from the TUI. The server owns execution truth;
//! this module only (a) creates the execution + its `tui_remote_*` bookmark
//! via the shared remote-session path, (b) registers a
//! [`SubtaskRef`] on the PARENT session's harness runtime, and (c) projects
//! status/summary/tokens back over plain HTTP reads. The
//! `subtask_registry` helpers shape the local bookkeeping; this module owns
//! the I/O around them.

use super::client::{normalize_url, ServerClient};
use anyhow::{Context, Result};
use base64::Engine;
use opencoder_core::{
    fleet::{CreateExecution, ExecutionIndex, ExecutionStatus, MessageCursor, MessagePage},
    harness::{RemoteSession, ServerCapability, SubtaskRef},
    message::now_ms,
    Config, Message,
};
use opencoder_llm::ChatStream;
use opencoder_store::Store;
use serde_json::Value;
use std::{path::Path, sync::Arc};

/// Sum of per-message persisted usage (mirrors the replay cost fold).
pub(crate) fn sum_message_usage(messages: &[Message]) -> u64 {
    messages
        .iter()
        .map(|message| message.usage.total_tokens)
        .sum()
}

/// Sum of `total_tokens` across `llm_usage` frames (mirrors the live cost
/// fold; covers kinds that expose events but no messages).
pub(crate) fn sum_event_usage(frames: &[Value]) -> u64 {
    frames
        .iter()
        .filter(|frame| frame["kind"] == "llm_usage")
        .filter_map(|frame| frame["data"]["total_tokens"].as_u64())
        .sum()
}

/// Parse an `@<capability> <question>` composer mention. Returns the matched
/// capability id (case-sensitive, `[A-Za-z0-9][A-Za-z0-9_-]*`) and the
/// non-empty trimmed question. `@file` mentions and plain text fall through
/// as `None`.
pub(crate) fn parse_mention(text: &str, known: &[ServerCapability]) -> Option<(String, String)> {
    let body = text.strip_prefix('@')?;
    let (token, remainder) = body.split_once(char::is_whitespace)?;
    if !valid_capability_token(token) {
        return None;
    }
    let question = remainder.trim();
    if question.is_empty() {
        return None;
    }
    known
        .iter()
        .find(|capability| capability.id == token)
        .map(|capability| (capability.id.clone(), question.to_string()))
}

fn valid_capability_token(token: &str) -> bool {
    let mut chars = token.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Current lifecycle status of a server execution.
pub(crate) async fn fetch_status(client: &ServerClient, id: &str) -> Result<ExecutionStatus> {
    let index: ExecutionIndex = client.get(&format!("/api/executions/{id}/index")).await?;
    Ok(index.status)
}

/// Bounded result summary of a finished execution (None when the server has
/// no conclusion yet).
pub(crate) async fn fetch_summary(client: &ServerClient, id: &str) -> Result<Option<String>> {
    let result: Value = client.get(&format!("/api/executions/{id}/result")).await?;
    Ok(result["summary"]
        .as_str()
        .map(crate::subtask_registry::bound_summary))
}

/// Aggregated sub-execution token usage. Primary source: paged messages and
/// their persisted usage. Kinds without messages fall back to paging the
/// event stream and summing `llm_usage` frames; the two paths should agree,
/// so the larger (non-zero) total wins.
pub(crate) async fn fetch_tokens(client: &ServerClient, id: &str) -> Result<u64> {
    let mut total = sum_message_usage(&paged_messages(client, id).await?);
    if total == 0 {
        total = total.max(sum_event_usage(&paged_events(client, id).await?));
    }
    Ok(total)
}

/// Ask the server to interrupt a subtask; the response body is ignored.
pub(crate) async fn cancel(client: &ServerClient, id: &str) -> Result<()> {
    client
        .command(id, "interrupt", serde_json::json!({}))
        .await?;
    Ok(())
}

async fn paged_messages(client: &ServerClient, id: &str) -> Result<Vec<Message>> {
    let mut cursor = MessageCursor::default();
    let mut result = Vec::new();
    let mut buffer = Vec::new();
    let mut sequence = None;
    loop {
        let page: MessagePage = serde_json::from_value(
            client
                .get(&format!(
                    "/api/executions/{id}/messages?seq={}&offset={}",
                    cursor.seq, cursor.offset
                ))
                .await?,
        )?;
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

async fn paged_events(client: &ServerClient, id: &str) -> Result<Vec<Value>> {
    let mut frames = Vec::new();
    let mut after = 0;
    loop {
        let page: Value = client
            .get(&format!("/api/executions/{id}/events-page?after={after}"))
            .await?;
        let events = page["events"]
            .as_array()
            .context("invalid Server event page")?;
        for frame in events {
            let seq = frame["seq"].as_i64().context("missing event cursor")?;
            anyhow::ensure!(seq > after, "Server event cursor did not advance");
            after = seq;
            frames.push(frame.clone());
        }
        if page["more"] != true {
            break;
        }
        anyhow::ensure!(!events.is_empty(), "empty Server event page with more=true");
    }
    Ok(frames)
}

/// Result of a one-shot subtask dispatch: the server execution id (which is
/// also the local bookmark session id).
pub(crate) struct DispatchOutcome {
    pub execution_id: String,
}

/// Create the subtask's server execution (plus its bookmark session) and
/// register it on the parent session's runtime as a pending `SubtaskRef`.
/// The bookmark is persisted BEFORE the execution create so a lost response
/// still leaves a recoverable `initial_input`, exactly like task switching.
pub(crate) async fn dispatch(
    store: &Arc<dyn Store>,
    config: &Config,
    client_llm: Arc<dyn ChatStream>,
    workdir: &Path,
    capability: ServerCapability,
    question: &str,
    parent_session: &str,
) -> Result<DispatchOutcome> {
    let client = ServerClient::configured(config)?;
    let input = serde_json::json!({"prompt": question, "images": [], "literal_mentions": true});
    let remote = RemoteSession {
        server_url: normalize_url(&config.opencoder_server.url)?,
        capability,
        created: false,
        initial_input: Some(input.clone()),
    };
    let session =
        super::session::create(remote, config.clone(), client_llm, store.clone(), workdir).await?;
    // Rebuild the create request from the persisted bookmark so kind/target
    // cannot drift from what session::create validated.
    let bookmark = session
        .harness
        .remote
        .as_ref()
        .context("remote task bookmark missing")?;
    let request = CreateExecution {
        id: session.id.clone(),
        kind: bookmark.capability.kind,
        target: Some(bookmark.capability.target.clone()),
        input,
        node_id: None,
    };
    client
        .post("/api/executions", &serde_json::to_value(&request)?)
        .await?;
    let mut runtime = store
        .harness_runtime(&session.id)
        .await?
        .context("remote task bookmark missing")?;
    if let Some(bookmark) = runtime.remote.as_mut() {
        bookmark.created = true;
        bookmark.initial_input = None;
    }
    store.set_harness_runtime(&session.id, &runtime).await?;
    let mut parent = store
        .harness_runtime(parent_session)
        .await?
        .context("parent session runtime missing")?;
    parent.subtasks = crate::subtask_registry::register(
        &parent.subtasks,
        SubtaskRef {
            execution_id: session.id.clone(),
            kind: bookmark.capability.kind.prefix().to_string(),
            capability_id: bookmark.capability.id.clone(),
            question: question.to_string(),
            status: "pending".into(),
            result_summary: None,
            tokens: 0,
            created_at: now_ms(),
        },
    );
    store.set_harness_runtime(parent_session, &parent).await?;
    Ok(DispatchOutcome {
        execution_id: session.id,
    })
}

/// Spawn `dispatch` on tokio, mirroring `super::selection::request`: the
/// composer stays responsive and polls the returned one-shot channel.
pub(crate) fn request_dispatch(
    store: Arc<dyn Store>,
    config: Config,
    client_llm: Arc<dyn ChatStream>,
    workdir: std::path::PathBuf,
    capability: ServerCapability,
    question: String,
    parent_session: String,
) -> tokio::sync::oneshot::Receiver<Result<DispatchOutcome>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(
            dispatch(
                &store,
                &config,
                client_llm,
                &workdir,
                capability,
                &question,
                &parent_session,
            )
            .await,
        );
    });
    rx
}

/// The parent session's subtask registry (empty when no runtime exists).
pub(crate) async fn load_registry(
    store: &Arc<dyn Store>,
    session_id: &str,
) -> Result<Vec<SubtaskRef>> {
    Ok(store
        .harness_runtime(session_id)
        .await?
        .map(|runtime| runtime.subtasks)
        .unwrap_or_default())
}

/// Read-modify-write the parent runtime's `subtasks` field only; every other
/// harness field keeps its stored value.
pub(crate) async fn save_registry(
    store: &Arc<dyn Store>,
    session_id: &str,
    refs: &[SubtaskRef],
) -> Result<()> {
    let mut runtime = store
        .harness_runtime(session_id)
        .await?
        .context("session runtime missing")?;
    runtime.subtasks = refs.to_vec();
    store.set_harness_runtime(session_id, &runtime).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::ExecutionKind;
    use serde_json::json;

    fn message_with_tokens(id: &str, tokens: u64) -> Message {
        let mut message = Message::assistant(id);
        message.usage.total_tokens = tokens;
        message
    }

    #[test]
    fn sum_message_usage_folds_all_messages() {
        let messages = [
            message_with_tokens("m1", 100),
            Message::user("m2", "hi"),
            message_with_tokens("m3", 25),
        ];
        assert_eq!(sum_message_usage(&messages), 125);
        assert_eq!(sum_message_usage(&[]), 0);
    }

    #[test]
    fn sum_event_usage_counts_llm_usage_frames_only() {
        let frames = [
            json!({"kind":"llm_usage","data":{"total_tokens":30}}),
            json!({"kind":"text_delta","data":{"total_tokens":999}}),
            json!({"kind":"llm_usage","data":{"total_tokens":12}}),
            json!({"kind":"llm_usage","data":{}}),
        ];
        assert_eq!(sum_event_usage(&frames), 42);
        assert_eq!(sum_event_usage(&[]), 0);
    }

    #[test]
    fn parse_mention_matches_known_capability_and_question() {
        let known = vec![ServerCapability {
            id: "ops_agent-1".into(),
            kind: ExecutionKind::Agent,
            target: "act".into(),
            summary: String::new(),
        }];
        assert_eq!(
            parse_mention("@ops_agent-1   check the logs ", &known),
            Some(("ops_agent-1".into(), "check the logs".into()))
        );
        assert_eq!(parse_mention("@ops_agent-1", &known), None);
        assert_eq!(parse_mention("@other what?", &known), None);
        assert_eq!(parse_mention("@OPS_AGENT-1 what?", &known), None);
        assert_eq!(parse_mention("plain text", &known), None);
        assert_eq!(parse_mention("@notes.md summarize", &known), None);
        assert_eq!(parse_mention("@-dash leading", &known), None);
        assert_eq!(parse_mention("@", &known), None);
    }
}
