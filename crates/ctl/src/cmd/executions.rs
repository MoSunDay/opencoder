//! Executions domain: the control plane's execution registry — list/create,
//! inspect, runtime commands, event streams (SSE + paged), per-event payload
//! and detail-field slices, messages/todo/project/team projections, and
//! binary artifact download. Pure `plan()` mapping plus two streaming
//! special cases (`events`, `artifact`); everything else buffers via
//! `exec_plan`.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::{exec_plan, exec_stream, open_stream, raw::parse_body, stream_failure};
use crate::ctx::Ctx;
use crate::http::RequestPlan;
use crate::out;

#[derive(Subcommand, Debug)]
pub enum ExecCmd {
    /// List executions, newest first (keyset paginated).
    List {
        /// Filter by node id.
        #[arg(long)]
        node_id: Option<String>,
        /// Filter by execution kind (agent/dag/project/...).
        #[arg(long)]
        kind: Option<String>,
        /// Page size; the server default applies when omitted.
        #[arg(long)]
        limit: Option<u32>,
        /// Cursor: creation timestamp of the last seen row (pair with --cursor-id).
        #[arg(long)]
        cursor_created_at: Option<i64>,
        /// Cursor: execution id of the last seen row (pair with --cursor-created-at).
        #[arg(long)]
        cursor_id: Option<String>,
    },
    /// Create an execution from a full JSON body ({"id","kind","node_id",...}).
    Create {
        /// Request body: inline JSON or @file with JSON content.
        #[arg(long, required = true)]
        json: String,
    },
    /// Inspect one execution by id.
    Get { id: String },
    /// Send a runtime command (cancel/steer/...) to an execution.
    #[command(alias = "command")]
    Cmd {
        id: String,
        /// Command action, e.g. cancel or steer.
        #[arg(long)]
        action: String,
        /// Command input value: inline JSON or @file; defaults to null.
        #[arg(long)]
        json: Option<String>,
    },
    /// Follow the execution event stream (SSE, printed as JSON frames).
    Events {
        id: String,
        /// Start after this event sequence.
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    /// Read one buffered page of events (JSON snapshot, not a stream).
    EventsPage {
        id: String,
        /// Page starts after this event sequence.
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    /// Read one event's raw payload by sequence, optionally byte-offset.
    Payload {
        id: String,
        /// Event sequence number (1-based).
        seq: i64,
        /// Byte offset into the payload.
        #[arg(long)]
        offset: Option<u64>,
    },
    /// Read a named detail field of an execution (paged by byte offset).
    #[command(alias = "detail-field")]
    Field {
        id: String,
        /// Field name (letters/digits/._-), e.g. stdout or stderr.
        #[arg(long)]
        field: String,
        /// Byte offset into the field value.
        #[arg(long)]
        offset: Option<u64>,
    },
    /// Read the execution's chat messages (cursor: seq + offset).
    Messages {
        id: String,
        /// Message sequence to start from.
        #[arg(long)]
        seq: Option<i64>,
        /// Byte offset within the message at `seq`.
        #[arg(long)]
        offset: Option<u64>,
    },
    /// Read the execution's TODO items (cursor: after_ordinal).
    TodoItems {
        id: String,
        /// Only items with a greater ordinal.
        #[arg(long)]
        after_ordinal: Option<i64>,
    },
    /// Read the execution's project run versions (cursor: before_version).
    ProjectRuns {
        id: String,
        /// Only versions strictly before this one.
        #[arg(long)]
        before_version: Option<i64>,
    },
    /// Read the execution's team turns (cursor: after_turn).
    TeamTurns {
        id: String,
        /// Only turns after this index.
        #[arg(long)]
        after_turn: Option<u32>,
    },
    /// Download a DAG/project execution artifact as a binary stream.
    Artifact {
        id: String,
        /// Workflow step id the artifact belongs to.
        #[arg(long)]
        step: String,
        /// Artifact file name; server defaults to output.txt.
        #[arg(long)]
        file: Option<String>,
        /// Destination path, or `-` for raw bytes on stdout.
        #[arg(short = 'o', long, default_value = "-")]
        out: std::path::PathBuf,
    },
}

/// Pure mapping: subcommand → request plan. No I/O except reading an
/// explicit `@file` body; invalid pairs/bodies are rejected here.
pub fn plan(sub: &ExecCmd) -> Result<RequestPlan> {
    Ok(match sub {
        ExecCmd::List {
            node_id,
            kind,
            limit,
            cursor_created_at,
            cursor_id,
        } => {
            anyhow::ensure!(
                cursor_created_at.is_some() == cursor_id.is_some(),
                "--cursor-created-at and --cursor-id must be provided together"
            );
            RequestPlan::get("/api/executions")
                .with_opt("node_id", node_id.clone())
                .with_opt("kind", kind.clone())
                .with_opt("limit", limit.map(|value| value.to_string()))
                .with_opt(
                    "cursor_created_at",
                    cursor_created_at.map(|v| v.to_string()),
                )
                .with_opt("cursor_id", cursor_id.clone())
        }
        ExecCmd::Create { json } => {
            RequestPlan::post("/api/executions").with_opt_body(parse_body(Some(json))?)
        }
        ExecCmd::Get { id } => RequestPlan::get(format!("/api/executions/{id}")),
        ExecCmd::Cmd { id, action, json } => RequestPlan::post(format!(
            "/api/executions/{id}/commands"
        ))
        .with_body(serde_json::json!({
            "action": action,
            "input": parse_body(json.as_deref())?.unwrap_or(serde_json::Value::Null),
        })),
        ExecCmd::Events { id, after } => RequestPlan::get(format!("/api/executions/{id}/events"))
            .with("after", after.to_string()),
        ExecCmd::EventsPage { id, after } => {
            RequestPlan::get(format!("/api/executions/{id}/events-page"))
                .with("after", after.to_string())
        }
        ExecCmd::Payload { id, seq, offset } => {
            RequestPlan::get(format!("/api/executions/{id}/events/{seq}/payload"))
                .with_opt("offset", offset.map(|value| value.to_string()))
        }
        ExecCmd::Field { id, field, offset } => {
            RequestPlan::get(format!("/api/executions/{id}/detail-field"))
                .with("field", field.clone())
                .with_opt("offset", offset.map(|value| value.to_string()))
        }
        ExecCmd::Messages { id, seq, offset } => {
            RequestPlan::get(format!("/api/executions/{id}/messages"))
                .with_opt("seq", seq.map(|value| value.to_string()))
                .with_opt("offset", offset.map(|value| value.to_string()))
        }
        ExecCmd::TodoItems { id, after_ordinal } => {
            RequestPlan::get(format!("/api/executions/{id}/todo-items"))
                .with_opt("after_ordinal", after_ordinal.map(|v| v.to_string()))
        }
        ExecCmd::ProjectRuns { id, before_version } => {
            RequestPlan::get(format!("/api/executions/{id}/project-runs"))
                .with_opt("before_version", before_version.map(|v| v.to_string()))
        }
        ExecCmd::TeamTurns { id, after_turn } => {
            RequestPlan::get(format!("/api/executions/{id}/team-turns"))
                .with_opt("after_turn", after_turn.map(|value| value.to_string()))
        }
        ExecCmd::Artifact { id, step, file, .. } => {
            RequestPlan::get(format!("/api/executions/{id}/artifact"))
                .with("step", step.clone())
                .with_opt("file", file.clone())
        }
    })
}

pub async fn run(ctx: &Ctx, sub: ExecCmd) -> Result<i32> {
    if let ExecCmd::Artifact { out: dest, .. } = &sub {
        return run_artifact(ctx, plan(&sub)?, dest).await;
    }
    if let ExecCmd::Events { .. } = &sub {
        return run_events(ctx, plan(&sub)?).await;
    }
    exec_plan(ctx, plan(&sub)?).await
}

/// `events` special case: no buffered JSON on stdout; frames stream as one
/// JSON line each via the shared streaming executor until the server ends
/// the stream or the user hits Ctrl-C.
async fn run_events(ctx: &Ctx, request: RequestPlan) -> Result<i32> {
    exec_stream(ctx, request).await
}

/// `artifact` special case: binary body streamed to `dest` (`-` = raw
/// stdout for piping); rejections keep the stderr JSON + exit-code contract.
async fn run_artifact(ctx: &Ctx, request: RequestPlan, dest: &std::path::Path) -> Result<i32> {
    let response = match open_stream(ctx, &request).await {
        Ok(response) => response,
        Err(error) => {
            out::fail_transport(&format!("{error:#}"));
            return Ok(1);
        }
    };
    if !response.status().is_success() {
        return Ok(stream_failure(response).await);
    }
    match crate::http::save_body(response, dest).await {
        Ok(()) => Ok(0),
        Err(error) => {
            out::fail_transport(&format!("{error:#}"));
            Ok(1)
        }
    }
}
