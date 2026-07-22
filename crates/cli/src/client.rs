//! `opencode client`: headless remote client. Resolves a session, posts a
//! prompt to a remote `opencode server`, and streams the result back to stdout
//! by decoding the server's SSE `/events` stream. The client stores nothing
//! locally and calls no LLM — it is a thin shell over the server.

use anyhow::{anyhow, Result};
use opencoder_client::Remote;
use opencoder_session::SessionEvent;

use crate::run::print_event;

/// Resolve the client bearer token: `--token` flag, then
/// `OPENCODER_SERVER_TOKEN` env. Unlike the server, the client does NOT
/// auto-generate a token (a random token could never authenticate).
pub fn resolve_token(token: Option<String>) -> Result<String> {
    if let Some(t) = token {
        return Ok(t);
    }
    std::env::var("OPENCODER_SERVER_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow!("no token: pass --token <T> or set OPENCODER_SERVER_TOKEN"))
}

pub async fn client_run(
    remote: String,
    token: Option<String>,
    session: Option<String>,
    continue_: bool,
    prompt: String,
) -> Result<()> {
    let token = resolve_token(token)?;
    let client = Remote::new(&remote, &token)?;

    // Resolve the target session: explicit id > --continue (most recent) >
    // create a fresh one.
    let session_id = if let Some(id) = session {
        id
    } else if continue_ {
        let list = client.list_sessions().await?;
        list.first()
            .and_then(|item| item.get("id"))
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("no sessions on the server to --continue"))?
    } else {
        client.create_session(None, None).await?
    };

    // Snapshot the current event cursor so we only stream events produced by
    // THIS prompt (not the whole prior transcript).
    let after = client.last_event_seq(&session_id).await?;

    eprintln!("\n\x1b[1muser\x1b[0m: {}\n", prompt.trim_end());
    let _admitted = client
        .post_prompt(&session_id, &prompt, None, None, None)
        .await?;

    let mut rx = client.events(&session_id, after)?;
    while let Some(evt) = rx.recv().await {
        // TranscriptReset carries no messages on the wire: pull a fresh
        // transcript snapshot from the server (rebuild path for compaction).
        if evt.kind == "transcript_reset" {
            let _ = client.get_messages(&session_id).await;
            // headless output is append-only; nothing to redraw here.
            continue;
        }
        let Some(ev) = SessionEvent::from_sse(&evt.kind, evt.data) else {
            // unknown event kind — ignore rather than abort the stream
            continue;
        };
        print_event(&ev);
        match &ev {
            SessionEvent::Done => break,
            SessionEvent::Error(e) => return Err(anyhow!("{e}")),
            _ => {}
        }
    }

    eprintln!("\n\x1b[2m[remote session {}]\x1b[0m", session_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_token;

    #[test]
    fn resolve_token_param_returns_ok() {
        assert_eq!(resolve_token(Some("explicit".into())).unwrap(), "explicit");
    }
}
