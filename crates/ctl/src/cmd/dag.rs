//! `dag` domain: stored DAG definitions (`/api/dag/defs`) and dispatched
//! runs (`/api/dag/runs`). Pure `plan()` mapping only; execution goes
//! through the shared `exec_plan` transport — except `runs events`, which
//! streams the SSE endpoint instead of buffering.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::{exec_plan, exec_stream, raw::parse_body};
use crate::ctx::Ctx;
use crate::http::RequestPlan;

/// `dag defs ...` — stored DAG definitions.
#[derive(Subcommand, Debug)]
pub enum DagDefsCmd {
    /// List every DAG definition.
    #[command(alias = "ls")]
    List,
    /// Create/replace a definition (body: a DagSpec or {"spec": DagSpec}).
    Put {
        /// Definition body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Read one definition by id.
    Get { id: String },
    /// Delete one definition by id.
    Delete { id: String },
}

/// `dag runs ...` — dispatched DAG executions.
#[derive(Subcommand, Debug)]
pub enum DagRunsCmd {
    /// List runs (no server-side filters on this endpoint).
    #[command(alias = "ls")]
    List,
    /// Inspect one run: status, steps, artifacts.
    Get { id: String },
    /// Stream run events: one compact JSON line per SSE frame.
    Events {
        id: String,
        /// Resume after this event sequence number.
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    /// Cancel a running DAG.
    Cancel { id: String },
}

#[derive(Subcommand, Debug)]
pub enum DagCmd {
    /// Definitions: list / put / get / delete.
    #[command(subcommand)]
    Defs(DagDefsCmd),
    /// Dispatch a definition into a new run.
    Dispatch {
        /// Definition id.
        id: String,
        /// Dispatch options ({"id","node_id"}); `{}` when omitted.
        #[arg(long)]
        json: Option<String>,
    },
    /// Runs: list / get / events / cancel.
    #[command(subcommand)]
    Runs(DagRunsCmd),
}

/// Pure mapping: subcommand → request plan (no side effects). The server's
/// list/dispatch handlers take no query extractors, so no invented filters.
pub fn plan(sub: &DagCmd) -> Result<RequestPlan> {
    Ok(match sub {
        DagCmd::Defs(DagDefsCmd::List) => RequestPlan::get("/api/dag/defs"),
        DagCmd::Defs(DagDefsCmd::Put { json }) => {
            RequestPlan::post("/api/dag/defs").with_opt_body(parse_body(Some(json.as_str()))?)
        }
        DagCmd::Defs(DagDefsCmd::Get { id }) => RequestPlan::get(format!("/api/dag/defs/{id}")),
        DagCmd::Defs(DagDefsCmd::Delete { id }) => {
            RequestPlan::delete(format!("/api/dag/defs/{id}"))
        }
        // The dispatch handler requires a JSON body; an omitted --json
        // degrades to `{}` (server then mints the run id).
        DagCmd::Dispatch { id, json } => RequestPlan::post(format!("/api/dag/defs/{id}/dispatch"))
            .with_body(parse_body(json.as_deref())?.unwrap_or_else(|| serde_json::json!({}))),
        DagCmd::Runs(DagRunsCmd::List) => RequestPlan::get("/api/dag/runs"),
        DagCmd::Runs(DagRunsCmd::Get { id }) => RequestPlan::get(format!("/api/dag/runs/{id}")),
        DagCmd::Runs(DagRunsCmd::Events { id, after }) => {
            RequestPlan::get(format!("/api/dag/runs/{id}/events")).with("after", after.to_string())
        }
        DagCmd::Runs(DagRunsCmd::Cancel { id }) => {
            RequestPlan::post(format!("/api/dag/runs/{id}/cancel"))
        }
    })
}

/// Execute a plan; `runs events` streams SSE frames instead of buffering.
pub async fn run(ctx: &Ctx, sub: DagCmd) -> Result<i32> {
    let request = plan(&sub)?;
    if matches!(sub, DagCmd::Runs(DagRunsCmd::Events { .. })) {
        return exec_stream(ctx, request).await;
    }
    exec_plan(ctx, request).await
}
