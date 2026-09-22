//! Apply a report and acknowledge Host handoff without requeueing to ourselves.
use super::report::CompleteReport;
use crate::AppState;
use axum::extract::ws::Message;
use futures::{Sink, SinkExt};
use opencoder_core::fleet::{ExecutionCommand, NodeOperation, ServerFrame};

pub(super) async fn finish<S>(
    state: &AppState,
    id: &str,
    generation: &str,
    sequence: u64,
    complete: &CompleteReport,
    writer: &mut S,
) -> anyhow::Result<bool>
where
    S: Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    if !super::handoff_report::apply(state, id, generation, sequence, complete).await? {
        return Ok(false);
    }
    if complete.initial && generation.starts_with("host-") {
        let server = state
            .lifecycle
            .platform
            .get()
            .map(|platform| platform.release_id.as_str())
            .unwrap_or("legacy");
        let frame = ServerFrame::Call {
            request_id: ulid::Ulid::new().to_string(),
            operation: NodeOperation::Maintenance {
                command: ExecutionCommand {
                    action: "host_handoff_ready".into(),
                    input: serde_json::json!({"server":server}),
                },
            },
        };
        // Applying the report makes this node schedulable. Concurrent RPCs
        // may already fill the outgoing queue, whose sole consumer is our
        // socket loop. Awaiting an enqueue here would deadlock that consumer.
        let text = serde_json::to_string(&frame)?;
        anyhow::ensure!(
            text.len() <= opencoder_core::fleet::MAX_FRAME_BYTES,
            "server frame exceeds limit"
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            writer.send(Message::Text(text)),
        )
        .await??;
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
