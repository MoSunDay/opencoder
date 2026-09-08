//! Session domain. Two transport classes, kept explicit in `plan()`:
//!
//! * native control-plane routes — `GET/POST /api/sessions`,
//!   `GET /api/sessions/{id}/events` (SSE, `?after=`) and the compat
//!   `GET /api/sessions/{id}/task`;
//! * everything else is the relay surface: control's fallback forwards any
//!   `METHOD /api/sessions/{id}/<tail>` (with optional JSON body and query)
//!   as an `http` command to the owning node's session runtime. Tails mirror
//!   the authoritative worker routes in `crates/web/src/lib.rs`:
//!   messages / prompt / agent / model / interrupt / fork / compact /
//!   handoff / skill / questions[/{qid}/answer|skip] / inputs[/reorder|/{seq}]
//!   / annotation / autopilot / subagents[/{task}/steer], plus
//!   `DELETE /api/sessions/{id}`.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::raw::parse_body;
use crate::cmd::{exec_plan, exec_stream};
use crate::ctx::Ctx;
use crate::http::RequestPlan;

#[derive(Subcommand, Debug)]
pub enum SessionCmd {
    /// List sessions across the fleet (native control route).
    List,
    /// Create a session; body `{"agent":..,"model":..}` (native route).
    Create {
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Fetch one session's meta + messages (relay GET).
    Get {
        /// Session id.
        id: String,
    },
    /// Delete a session (cascades messages/inputs/events; relay DELETE).
    Delete {
        /// Session id.
        id: String,
    },
    /// Fetch the full message transcript (relay GET; no query params).
    Messages {
        /// Session id.
        id: String,
    },
    /// Admit a prompt; body `{"prompt":..,"images":[..],..}` (relay POST).
    Prompt {
        /// Session id.
        id: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Stream session events as SSE JSON lines (native route, --after cursor).
    Events {
        /// Session id.
        id: String,
        /// Replay events after this cursor (default 0 = from the start).
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    /// Switch the session agent; body `{"value":"act"|"plan"}` (relay POST).
    Agent {
        /// Session id.
        id: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Switch the session model; body `{"value":"provider/model"}` (relay).
    Model {
        /// Session id.
        id: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Interrupt the running drain (relay POST, no body).
    Interrupt {
        /// Session id.
        id: String,
    },
    /// Fork the session into a new id (relay POST, no body).
    Fork {
        /// Session id.
        id: String,
    },
    /// Compact the transcript now (relay POST, no body).
    Compact {
        /// Session id.
        id: String,
    },
    /// Execution handoff to act; optional body `{"extra":..}` (relay POST).
    Handoff {
        /// Session id.
        id: String,
        /// Optional request body: inline JSON or @file.
        #[arg(long)]
        json: Option<String>,
    },
    /// Set/clear the active skill; body `{"skill":name|null}` (relay POST).
    Skill {
        /// Session id.
        id: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// List waiting questions (relay GET).
    Questions {
        /// Session id.
        id: String,
    },
    /// Answer a waiting question; body `{"answer":..}` (relay POST).
    Answer {
        /// Session id.
        id: String,
        /// Question call id.
        qid: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Skip a waiting question (relay POST, no body).
    Skip {
        /// Session id.
        id: String,
        /// Question call id.
        qid: String,
    },
    /// List pending inputs; --delivery steer|queue, default steer (relay GET).
    Inputs {
        /// Session id.
        id: String,
        /// Filter by delivery: "steer" (default) or "queue".
        #[arg(long)]
        delivery: Option<String>,
    },
    /// Swap two pending inputs; body `{"a":seq,"b":seq}` (relay POST).
    InputReorder {
        /// Session id.
        id: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Delete a pending input by seq before the drain consumes it (relay).
    InputDelete {
        /// Session id.
        id: String,
        /// Pending input seq.
        seq: i64,
    },
    /// Set/clear the requirement annotation; optional `{"text":..}` (relay).
    Annotation {
        /// Session id.
        id: String,
        /// Optional request body: inline JSON or @file (absent = clear).
        #[arg(long)]
        json: Option<String>,
    },
    /// Set/clear autopilot override; optional `{"mode":..}` (relay POST).
    Autopilot {
        /// Session id.
        id: String,
        /// Optional request body: inline JSON or @file (absent/null = clear).
        #[arg(long)]
        json: Option<String>,
    },
    /// List subagent tasks of the session (relay GET).
    Subagents {
        /// Session id.
        id: String,
    },
    /// Steer a running subagent; body `{"prompt":..}` (relay POST).
    Steer {
        /// Session id.
        id: String,
        /// Subagent task id.
        task: String,
        /// Request body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Get the node task that owns this session (native compat route).
    Task {
        /// Session id (= execution id).
        id: String,
    },
}

/// Pure mapping subcommand → request plan. No I/O; `--json` strings are
/// parsed here so parse failures surface before any socket is opened.
pub fn plan(sub: &SessionCmd) -> Result<RequestPlan> {
    const SESSIONS: &str = "/api/sessions";
    Ok(match sub {
        // ── native control-plane routes ─────────────────────────────
        SessionCmd::List => RequestPlan::get(SESSIONS),
        SessionCmd::Create { json } => {
            RequestPlan::post(SESSIONS).with_body(parse_body(Some(json))?.expect("required body"))
        }
        SessionCmd::Events { id, after } => {
            RequestPlan::get(format!("{SESSIONS}/{id}/events")).with("after", after.to_string())
        }
        SessionCmd::Task { id } => RequestPlan::get(format!("{SESSIONS}/{id}/task")),
        // ── relay surface: control forwards <method> /{tail} to the node ──
        SessionCmd::Get { id } => RequestPlan::get(format!("{SESSIONS}/{id}")),
        SessionCmd::Delete { id } => RequestPlan::delete(format!("{SESSIONS}/{id}")),
        SessionCmd::Messages { id } => RequestPlan::get(format!("{SESSIONS}/{id}/messages")),
        SessionCmd::Prompt { id, json } => RequestPlan::post(format!("{SESSIONS}/{id}/prompt"))
            .with_body(parse_body(Some(json))?.expect("required body")),
        SessionCmd::Agent { id, json } => RequestPlan::post(format!("{SESSIONS}/{id}/agent"))
            .with_body(parse_body(Some(json))?.expect("required body")),
        SessionCmd::Model { id, json } => RequestPlan::post(format!("{SESSIONS}/{id}/model"))
            .with_body(parse_body(Some(json))?.expect("required body")),
        SessionCmd::Interrupt { id } => RequestPlan::post(format!("{SESSIONS}/{id}/interrupt")),
        SessionCmd::Fork { id } => RequestPlan::post(format!("{SESSIONS}/{id}/fork")),
        SessionCmd::Compact { id } => RequestPlan::post(format!("{SESSIONS}/{id}/compact")),
        SessionCmd::Handoff { id, json } => RequestPlan::post(format!("{SESSIONS}/{id}/handoff"))
            .with_opt_body(parse_body(json.as_deref())?),
        SessionCmd::Skill { id, json } => RequestPlan::post(format!("{SESSIONS}/{id}/skill"))
            .with_body(parse_body(Some(json))?.expect("required body")),
        SessionCmd::Questions { id } => RequestPlan::get(format!("{SESSIONS}/{id}/questions")),
        SessionCmd::Answer { id, qid, json } => {
            RequestPlan::post(format!("{SESSIONS}/{id}/questions/{qid}/answer"))
                .with_body(parse_body(Some(json))?.expect("required body"))
        }
        SessionCmd::Skip { id, qid } => {
            RequestPlan::post(format!("{SESSIONS}/{id}/questions/{qid}/skip"))
        }
        SessionCmd::Inputs { id, delivery } => RequestPlan::get(format!("{SESSIONS}/{id}/inputs"))
            .with_opt("delivery", delivery.clone()),
        SessionCmd::InputReorder { id, json } => {
            RequestPlan::post(format!("{SESSIONS}/{id}/inputs/reorder"))
                .with_body(parse_body(Some(json))?.expect("required body"))
        }
        SessionCmd::InputDelete { id, seq } => {
            RequestPlan::delete(format!("{SESSIONS}/{id}/inputs/{seq}"))
        }
        SessionCmd::Annotation { id, json } => {
            RequestPlan::post(format!("{SESSIONS}/{id}/annotation"))
                .with_opt_body(parse_body(json.as_deref())?)
        }
        SessionCmd::Autopilot { id, json } => {
            RequestPlan::post(format!("{SESSIONS}/{id}/autopilot"))
                .with_opt_body(parse_body(json.as_deref())?)
        }
        SessionCmd::Subagents { id } => RequestPlan::get(format!("{SESSIONS}/{id}/subagents")),
        SessionCmd::Steer { id, task, json } => {
            RequestPlan::post(format!("{SESSIONS}/{id}/subagents/{task}/steer"))
                .with_body(parse_body(Some(json))?.expect("required body"))
        }
    })
}

/// Execute: `events` streams SSE to stdout line by line; everything else is
/// a single buffered request/response through the shared plan executor.
pub async fn run(ctx: &Ctx, sub: SessionCmd) -> Result<i32> {
    if matches!(sub, SessionCmd::Events { .. }) {
        let request = plan(&sub)?;
        return exec_stream(ctx, request).await;
    }
    exec_plan(ctx, plan(&sub)?).await
}
