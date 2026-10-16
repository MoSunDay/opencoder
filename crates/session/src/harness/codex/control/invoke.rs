//! The control invocation has its own owned process; the observer stays alive.
use super::journal::Request;
use crate::SessionState;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Reply {
    pub input_id: String,
    pub status: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub evidence: serde_json::Value,
}

pub(super) async fn call(session: &SessionState, request: &Request) -> Result<Reply> {
    let mut context: serde_json::Value =
        serde_json::from_str(&super::super::launch::context(session))?;
    context["control"] = serde_json::json!({
        "schema_version": request.schema_version, "action": request.action,
        "input_id": request.input_id, "parent_input_id": request.parent_input_id,
    });
    let mut process = super::super::process::spawn_with_context(
        session,
        serde_json::to_string(request)?,
        &[],
        Some(context.to_string()),
    )?;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let line = process
            .lines
            .next_line()
            .await?
            .context("external control receipt missing")?;
        ensure!(
            line.len() <= 1024 * 1024,
            "external control receipt too large"
        );
        let reply: Reply = serde_json::from_str(&line)?;
        ensure!(
            reply.input_id == request.input_id,
            "external control receipt identity mismatch"
        );
        ensure!(
            matches!(reply.status.as_str(), "accepted" | "rejected" | "pending"),
            "external control receipt status invalid"
        );
        ensure!(
            process.lines.next_line().await?.is_none(),
            "multiple external control receipts"
        );
        let (status, _) = process.finish().await?;
        ensure!(status.success(), "external control process failed");
        Ok(reply)
    })
    .await;
    match result {
        Ok(Ok(reply)) => Ok(reply),
        other => {
            process.stop().await?;
            other.context("external control observation timed out")?
        }
    }
}
