use crate::http::RequestPlan;
use anyhow::{ensure, Result};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Debug, Args)]
pub struct StepInput {
    pub id: String,
    pub step: String,
    #[arg(long)]
    pub instance: Option<usize>,
    #[arg(long)]
    pub expected_attempt: u64,
    #[arg(long)]
    pub operation_id: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Subcommand)]
pub enum StepsCmd {
    /// Read durable step state, or one operation receipt.
    Status {
        id: String,
        #[arg(long)]
        operation_id: Option<String>,
    },
    /// Stop only this attempt and hold its dependents.
    Pause(StepInput),
    /// Continue a stopped unresolved attempt with its original inputs.
    Resume(StepInput),
    /// Retry a stopped unresolved attempt; completed outputs remain immutable.
    Retry(StepInput),
    /// Explicitly settle an unresolved step as failed after stopping it.
    Fail(StepInput),
}

pub fn plan(command: &StepsCmd) -> Result<RequestPlan> {
    let (id, action, input) = match command {
        StepsCmd::Status { id, operation_id } => {
            (id, "step-status", json!({"operation_id":operation_id}))
        }
        _ => {
            let (action, args) = match command {
                StepsCmd::Pause(args) => ("step-pause", args),
                StepsCmd::Resume(args) => ("step-resume", args),
                StepsCmd::Retry(args) => ("step-retry", args),
                StepsCmd::Fail(args) => ("step-fail", args),
                _ => unreachable!(),
            };
            ensure!(!args.reason.trim().is_empty(), "reason is required");
            ensure!(
                !args.operation_id.is_empty()
                    && args.operation_id.len() <= 128
                    && args
                        .operation_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
                "invalid operation ID"
            );
            (
                &args.id,
                action,
                json!({"step":args.step,"instance":args.instance,"expected_attempt":args.expected_attempt,
                "operation_id":args.operation_id,"reason":args.reason}),
            )
        }
    };
    ensure!(
        !id.is_empty() && !id.contains(['/', '\\', '?', '#']),
        "invalid execution ID"
    );
    let input = if matches!(
        command,
        StepsCmd::Status {
            operation_id: None,
            ..
        }
    ) {
        json!({})
    } else {
        input
    };
    Ok(RequestPlan::post(format!("/api/executions/{id}/commands"))
        .with_body(json!({"action":action,"input":input})))
}
