//! SSE event consumption for `opencode client`, with transport-failure
//! recovery. The reconnect policy is factored into pure functions so the
//! decision table (reconnect budget + backoff schedule) is unit-testable
//! without a server.

use std::time::Duration;

use anyhow::{anyhow, Result};
use opencoder_client::Remote;
use opencoder_core::Role;
use opencoder_session::SessionEvent;

use crate::display::print_event;

/// Max reconnect attempts after a transport failure before giving up and
/// falling back to a transcript snapshot.
pub(crate) const MAX_RECONNECTS: u32 = 3;

/// Decision for "the SSE channel closed without a terminal event".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Transport hiccup: resubscribe after this many milliseconds.
    Reconnect { backoff_ms: u64 },
    /// Budget exhausted: snapshot the transcript and report failure.
    GiveUp,
}

/// Pure retry policy: 3 reconnects with exponential backoff 500ms/1s/2s.
pub fn on_stream_close(reconnects_used: u32) -> RetryDecision {
    if reconnects_used >= MAX_RECONNECTS {
        return RetryDecision::GiveUp;
    }
    let backoff_ms = match reconnects_used {
        0 => 500,
        1 => 1_000,
        _ => 2_000,
    };
    RetryDecision::Reconnect { backoff_ms }
}

/// Outcome of one SSE subscription attempt.
#[derive(Debug, PartialEq, Eq)]
enum StreamOutcome {
    /// Terminal `done` — the run finished.
    Done,
    /// Business `error` event — terminal, never retried.
    Error(String),
    /// Channel closed without a terminal event, or a `stream_error` marker
    /// arrived (transport failure — retryable).
    Lost,
}

/// Consume the session's SSE stream until a terminal event, recovering from
/// transport failures: on `Lost`, re-snapshot the server's event cursor,
/// resubscribe from there, and retry (≤ [`MAX_RECONNECTS`], exponential
/// backoff). A business `error` event returns `Err` immediately (no retry).
/// When the reconnect budget is exhausted, a final transcript snapshot is
/// printed (the last assistant message) so the user still gets the result,
/// then the failure is reported.
///
/// Known limitation (pre-existing, needs a server-side SSE keep-alive to
/// fix): if the server *restarts* and the resubscription succeeds but the
/// drain task for the session is gone, the stream stays open and silent —
/// the client then waits out the HTTP read timeout instead of retrying.
pub(crate) async fn stream_with_reconnect(
    client: &Remote,
    session_id: &str,
    mut after: i64,
) -> Result<()> {
    let mut reconnects_used: u32 = 0;
    loop {
        let outcome = consume_once(client, session_id, after).await;
        match outcome {
            StreamOutcome::Done => return Ok(()),
            StreamOutcome::Error(e) => return Err(anyhow!("{e}")),
            StreamOutcome::Lost => match on_stream_close(reconnects_used) {
                RetryDecision::Reconnect { backoff_ms } => {
                    reconnects_used += 1;
                    eprintln!(
                        "\x1b[2m[stream lost; reconnecting ({reconnects_used}/{MAX_RECONNECTS}) in {backoff_ms}ms]\x1b[0m"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    // Snapshot the server's cursor to resume without replays.
                    // If the snapshot fetch itself fails (server still down),
                    // fall through to another consume→policy round instead of
                    // surfacing the transport error: the retry budget governs.
                    if let Ok(seq) = client.last_event_seq(session_id).await {
                        after = seq;
                    }
                }
                RetryDecision::GiveUp => {
                    eprintln!(
                        "\x1b[33m[warning] stream lost after {MAX_RECONNECTS} reconnects; fetching transcript snapshot]\x1b[0m"
                    );
                    print_last_assistant(client, session_id).await;
                    return Err(anyhow!(
                        "stream lost after {MAX_RECONNECTS} reconnects; transcript snapshot printed above"
                    ));
                }
            },
        }
    }
}

/// One subscription attempt: consume events until `done`, a business `error`,
/// or the channel closes / a `stream_error` marker arrives.
async fn consume_once(client: &Remote, session_id: &str, after: i64) -> StreamOutcome {
    let mut rx = match client.events(session_id, after) {
        Ok(rx) => rx,
        Err(_) => return StreamOutcome::Lost,
    };
    loop {
        let evt = match rx.recv().await {
            Some(evt) => evt,
            // Channel closed without `done`: transport failure (server drop,
            // network cut, partial proxy flush). The retry loop decides.
            None => return StreamOutcome::Lost,
        };
        // Transport-failure marker synthesized by the client when the HTTP
        // stream itself errored — NOT a business error; retryable.
        if evt.kind == "stream_error" {
            return StreamOutcome::Lost;
        }
        // TranscriptReset carries no messages on the wire: pull a fresh
        // transcript snapshot from the server (rebuild path for compaction).
        if evt.kind == "transcript_reset" {
            let _ = client.get_messages(session_id).await;
            // headless output is append-only; nothing to redraw here.
            continue;
        }
        let Some(ev) = SessionEvent::from_sse(&evt.kind, evt.data) else {
            // unknown event kind — ignore rather than abort the stream
            continue;
        };
        if let SessionEvent::ToolStart { id, name, .. } = &ev {
            if name == "question" {
                eprintln!(
                    "\x1b[33m[question pending: answer in another terminal with: opencode client questions answer {session_id} {id} \"<answer>\"]\x1b[0m"
                );
            }
        }
        print_event(&ev);
        match ev {
            SessionEvent::Done => return StreamOutcome::Done,
            SessionEvent::Error(e) => return StreamOutcome::Error(e),
            _ => {}
        }
    }
}

/// Best-effort transcript fallback: print the last assistant message so the
/// user still sees the result after the stream died for good.
async fn print_last_assistant(client: &Remote, session_id: &str) {
    let Ok(messages) = client.get_messages(session_id).await else {
        eprintln!("(could not fetch transcript snapshot)");
        return;
    };
    let Some(last) = messages.iter().rev().find(|m| m.role == Role::Assistant) else {
        eprintln!("(no assistant message in transcript)");
        return;
    };
    let text: Vec<&str> = last.blocks.iter().filter_map(|b| b.as_text()).collect();
    println!("\n--- transcript snapshot (last assistant message) ---");
    println!("{}", text.join("\n"));
    println!("--- end snapshot ---");
}

#[cfg(test)]
mod tests {
    use super::{on_stream_close, RetryDecision};

    #[test]
    fn reconnects_with_exponential_backoff_then_gives_up() {
        // 1st..3rd failures reconnect with 500ms/1s/2s.
        assert_eq!(
            on_stream_close(0),
            RetryDecision::Reconnect { backoff_ms: 500 }
        );
        assert_eq!(
            on_stream_close(1),
            RetryDecision::Reconnect { backoff_ms: 1_000 }
        );
        assert_eq!(
            on_stream_close(2),
            RetryDecision::Reconnect { backoff_ms: 2_000 }
        );
        // Budget (MAX_RECONNECTS = 3) exhausted.
        assert_eq!(on_stream_close(3), RetryDecision::GiveUp);
        assert_eq!(on_stream_close(4), RetryDecision::GiveUp);
    }
}
