//! Apply a report and acknowledge Host handoff without requeueing to ourselves.
use super::report::CompleteReport;
use crate::AppState;
use axum::extract::ws::Message;
use opencoder_core::fleet::{ExecutionCommand, NodeOperation, ServerFrame};

pub(super) async fn acknowledge(
    sender: &tokio::sync::mpsc::Sender<super::socket_io::Acknowledgement>,
    report_id: u64,
    sequence: u64,
) -> anyhow::Result<()> {
    let frame = ServerFrame::Call {
        request_id: format!("report-ack-{report_id}-{sequence}"),
        operation: NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: opencoder_core::fleet::INDEX_REPORT_ACK_ACTION.into(),
                input: serde_json::json!({"report_id":report_id,"sequence":sequence}),
            },
        },
    };
    super::socket_io::acknowledge(sender, Message::Text(serde_json::to_string(&frame)?)).await
}

pub(super) async fn finish(
    state: &AppState,
    id: &str,
    generation: &str,
    sequence: u64,
    complete: &CompleteReport,
    acknowledgements: &tokio::sync::mpsc::Sender<super::socket_io::Acknowledgement>,
) -> anyhow::Result<bool> {
    if !super::handoff_report::apply(state, id, generation, sequence, complete).await? {
        return Ok(false);
    }
    if let Some(message) = handoff(state, generation, complete.initial)? {
        super::socket_io::acknowledge(acknowledgements, message).await?;
    }
    Ok(true)
}

fn handoff(state: &AppState, generation: &str, initial: bool) -> anyhow::Result<Option<Message>> {
    if initial && generation.starts_with("host-") {
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
        // This separate bounded channel bypasses a full RPC queue and waits
        // for the transport's actual write before finishing the report.
        let text = serde_json::to_string(&frame)?;
        anyhow::ensure!(
            text.len() <= opencoder_core::fleet::MAX_FRAME_BYTES,
            "server frame exceeds limit"
        );
        return Ok(Some(Message::Text(text)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
