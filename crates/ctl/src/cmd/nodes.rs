//! Nodes domain (control + compat routes, all verified against
//! `crates/control/src/routes.rs` and `api/compat/mod.rs`):
//!
//! * `GET /api/nodes` and `POST /api/nodes/{id}/maintenance` — native;
//! * `GET /api/models` / `GET /api/skills` — compat metadata queries
//!   (optional `?node_id=`; without it control picks an online agent node);
//! * `GET /api/nodes/{id}/dialogs` — compat dialog index;
//! * `POST /api/nodes/{id}/tasks` — compat task dispatch;
//! * `GET /api/nodes/tasks/{id}/events` — compat SSE stream (`?after=`);
//! * `POST /api/nodes/{node}/tasks/{id}/cancel` — compat cancel.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::raw::parse_body;
use crate::cmd::{exec_plan, exec_stream};
use crate::ctx::Ctx;
use crate::http::RequestPlan;

#[derive(Subcommand, Debug)]
pub enum NodesCmd {
    /// List registered fleet nodes (native control route).
    List,
    /// Run a maintenance command on a node; body is the execution command.
    Maintenance {
        /// Node id.
        id: String,
        /// Request body: inline JSON or @file (required).
        #[arg(long)]
        json: String,
    },
    /// Provider/model catalog from an agent node (compat route).
    Models {
        /// Query a specific node; default: control picks an online one.
        #[arg(long)]
        node: Option<String>,
    },
    /// Skill catalog from an agent node (compat route).
    Skills {
        /// Query a specific node; default: control picks an online one.
        #[arg(long)]
        node: Option<String>,
    },
    /// Dialog (session) index of one node (compat route).
    Dialogs {
        /// Node id.
        node: String,
    },
    /// Dispatch a task to a node; body `{"session_id"/"id":..,"prompt":..}`
    /// plus optional agent/model (compat route).
    TaskCreate {
        /// Node id.
        node: String,
        /// Request body: inline JSON or @file (required).
        #[arg(long)]
        json: String,
    },
    /// Cancel a task on its owning node (compat route).
    TaskCancel {
        /// Node id.
        node: String,
        /// Task (execution) id.
        task: String,
    },
    /// Stream a task's events as SSE JSON lines (compat route, --after).
    TaskEvents {
        /// Task (execution) id.
        task: String,
        /// Replay events after this cursor (default 0 = from the start).
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
}

/// Pure mapping subcommand → request plan. No I/O.
pub fn plan(sub: &NodesCmd) -> Result<RequestPlan> {
    Ok(match sub {
        NodesCmd::List => RequestPlan::get("/api/nodes"),
        NodesCmd::Maintenance { id, json } => {
            RequestPlan::post(format!("/api/nodes/{id}/maintenance"))
                .with_body(parse_body(Some(json))?.expect("required body"))
        }
        NodesCmd::Models { node } => {
            RequestPlan::get("/api/models").with_opt("node_id", node.clone())
        }
        NodesCmd::Skills { node } => {
            RequestPlan::get("/api/skills").with_opt("node_id", node.clone())
        }
        NodesCmd::Dialogs { node } => RequestPlan::get(format!("/api/nodes/{node}/dialogs")),
        NodesCmd::TaskCreate { node, json } => {
            RequestPlan::post(format!("/api/nodes/{node}/tasks"))
                .with_body(parse_body(Some(json))?.expect("required body"))
        }
        NodesCmd::TaskCancel { node, task } => {
            RequestPlan::post(format!("/api/nodes/{node}/tasks/{task}/cancel"))
        }
        NodesCmd::TaskEvents { task, after } => {
            RequestPlan::get(format!("/api/nodes/tasks/{task}/events"))
                .with("after", after.to_string())
        }
    })
}

/// Execute: `task-events` streams SSE to stdout line by line; everything else
/// is a single buffered request/response through the shared plan executor.
pub async fn run(ctx: &Ctx, sub: NodesCmd) -> Result<i32> {
    if matches!(sub, NodesCmd::TaskEvents { .. }) {
        let request = plan(&sub)?;
        return exec_stream(ctx, request).await;
    }
    exec_plan(ctx, plan(&sub)?).await
}
