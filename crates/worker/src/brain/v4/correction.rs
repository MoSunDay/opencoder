//! Bounded, durable correction of invalid model decisions.
use super::state;
use crate::{journal::Record, Worker};
use anyhow::{anyhow, Context, Result};
use opencoder_brain::layered;
use opencoder_core::{brain::layered::*, Config};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const MAX_ATTEMPTS: u64 = 3;
const BUDGET_MS: i64 = 300_000;

async fn mark(
    worker: &Worker,
    context: &LayeredContext,
    attempt: u64,
    deadline_ms: i64,
    feedback: &str,
) -> Result<()> {
    state::annotate(
        worker,
        &context.run_id,
        "layered_decision_attempt",
        json!({"generation":context.generation,"attempt":attempt,
            "deadline_ms":deadline_ms,"feedback":feedback}),
    )
    .await
}

pub(super) async fn decide(
    worker: &Worker,
    record: &Record,
    config: &Config,
    context: &LayeredContext,
    snapshot: &LayeredSnapshot,
    cancel: CancellationToken,
) -> Result<LayeredDecision> {
    let marker = &record.annotations["layered_decision_attempt"];
    let same = marker["generation"].as_u64() == Some(context.generation);
    let mut attempt = if same {
        marker["attempt"].as_u64().unwrap_or(0)
    } else {
        0
    };
    let mut feedback = if same {
        marker["feedback"].as_str().unwrap_or("").to_owned()
    } else {
        String::new()
    };
    let deadline_ms = if same {
        marker["deadline_ms"]
            .as_i64()
            .context("decision retry deadline missing")?
    } else {
        opencoder_core::message::now_ms() + BUDGET_MS
    };
    while attempt < MAX_ATTEMPTS {
        if cancel.is_cancelled() {
            anyhow::bail!("brain activation interrupted");
        }
        attempt += 1;
        // Persist before invoking the model. A crash consumes the attempt,
        // rather than resetting the budget and retrying indefinitely.
        mark(worker, context, attempt, deadline_ms, &feedback).await?;
        let mut corrected = context.clone();
        if !feedback.is_empty() {
            let run = corrected
                .run
                .as_mut()
                .context("decision context has no run state")?;
            run.error = Some(format!("Previous decision rejected: {feedback}. Correct only the decision; no capability was dispatched."));
        }
        let remaining = std::time::Duration::from_millis(
            deadline_ms.saturating_sub(opencoder_core::message::now_ms()) as u64,
        );
        if remaining.is_zero() {
            anyhow::bail!("brain decision exceeded five-minute correction budget");
        }
        // Let the container's cancellation path reap runc and remove its bundle.
        // Dropping that future via timeout would bypass its cleanup.
        let attempt_cancel = cancel.child_token();
        let deadline_cancel = attempt_cancel.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(remaining).await;
            deadline_cancel.cancel();
        });
        let result =
            super::super::container::layered(worker, config, &corrected, attempt_cancel).await;
        timer.abort();
        if opencoder_core::message::now_ms() >= deadline_ms {
            anyhow::bail!("brain decision exceeded five-minute correction budget");
        }
        let decision = match result {
            Ok(value) => value,
            Err(error) if error.to_string().contains("invalid layered decision") => {
                feedback = error.to_string();
                mark(worker, context, attempt, deadline_ms, &feedback).await?;
                continue;
            }
            Err(error) => return Err(error),
        };
        match layered::decide(
            snapshot,
            &context.request,
            &context.capabilities,
            &decision,
            opencoder_core::message::now_ms(),
        ) {
            Ok(_) => return Ok(decision),
            Err(error) => {
                feedback = error.to_string();
                mark(worker, context, attempt, deadline_ms, &feedback).await?;
            }
        }
    }
    Err(anyhow!(
        "layered decision invalid after {MAX_ATTEMPTS} attempts: {feedback}"
    ))
}
