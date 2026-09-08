//! `todo` domain: TODO workflow assets (env contexts, shared tools,
//! templates and versions) plus dispatched workflow runs. Pure `plan()`
//! mapping only; execution goes through the shared `exec_plan` transport —
//! except `workflows events`, which streams the SSE endpoint.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::{exec_plan, exec_stream, raw::parse_body};
use crate::ctx::Ctx;
use crate::http::RequestPlan;

/// `todo envs ...` — shared env contexts (`/api/todo/envs`).
#[derive(Subcommand, Debug)]
pub enum TodoEnvsCmd {
    /// List every env context.
    #[command(alias = "ls")]
    List,
    /// Create an env (body: {"name","description","tools","env_vars"}).
    Put {
        /// Env body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Read one env context.
    Get { name: String },
    /// Merge-patch an env (description/tools/env_vars; absent keys kept).
    Update {
        name: String,
        /// Patch body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Delete an env (whole directory).
    Delete { name: String },
}

/// `todo tools ...` — tool CLIs shared via the NFS share (`/api/todo/tools`).
#[derive(Subcommand, Debug)]
pub enum TodoToolsCmd {
    /// List share tools plus importable agent-bundled tools.
    #[command(alias = "ls")]
    List,
    /// Import an agent-bundled tool into the share
    /// (body: {"agent","version","tool"}).
    Import {
        /// Import body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
}

/// `todo templates ...` — templates, their `todo.json` meta and versions.
#[derive(Subcommand, Debug)]
pub enum TodoTemplatesCmd {
    /// List templates.
    #[command(alias = "ls")]
    List,
    /// Create a template (body: {"name","spec",...}; first version `v1`).
    Put {
        /// Template body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Read template meta plus the per-version env binding map.
    Get { name: String },
    /// Delete a whole template (all versions).
    Delete { name: String },
    /// Read the template's `todo.json` meta.
    GetMeta { name: String },
    /// Overwrite the template's `todo.json` meta.
    PutMeta {
        name: String,
        /// Meta body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Fork a version into the next one
    /// (body: {"source_version":"v1","note":".."}).
    NewVersion {
        name: String,
        /// Version body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Read a version's `context.json` (the WorkflowSpec).
    GetContext { name: String, version: String },
    /// Overwrite a version's `context.json`.
    PutContext {
        name: String,
        version: String,
        /// Spec body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Read a version's `env.json` binding.
    GetBinding { name: String, version: String },
    /// Overwrite a version's `env.json` binding.
    PutBinding {
        name: String,
        version: String,
        /// Binding body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// Delete one template version.
    DeleteVersion { name: String, version: String },
}

/// `todo workflows ...` — dispatched workflow runs (`/api/todo/workflows`).
#[derive(Subcommand, Debug)]
pub enum TodoWorkflowsCmd {
    /// List runs (no server-side filters on this endpoint).
    #[command(alias = "ls")]
    List,
    /// Inspect one run.
    Get { id: String },
    /// Ask a running workflow to stop at the next boundary.
    Interrupt { id: String },
    /// Continue an interrupted run.
    Resume { id: String },
    /// Stream run events: one compact JSON line per SSE frame.
    Events {
        id: String,
        /// Resume after this event sequence number.
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
}

#[derive(Subcommand, Debug)]
pub enum TodoCmd {
    /// Env contexts.
    #[command(subcommand)]
    Envs(TodoEnvsCmd),
    /// Shared tools.
    #[command(subcommand)]
    Tools(TodoToolsCmd),
    /// Templates and versions.
    #[command(subcommand)]
    Templates(TodoTemplatesCmd),
    /// Dispatch a template version into a new workflow run.
    Run {
        name: String,
        version: String,
        /// Dispatch options ({"id","node_id"}); `{}` when omitted.
        #[arg(long)]
        json: Option<String>,
    },
    /// Workflow runs: list / get / interrupt / resume / events.
    #[command(subcommand)]
    Workflows(TodoWorkflowsCmd),
}

/// Pure mapping: subcommand → request plan (no side effects). Every list
/// handler on these routes takes no query extractor, so no invented filters.
pub fn plan(sub: &TodoCmd) -> Result<RequestPlan> {
    Ok(match sub {
        TodoCmd::Envs(TodoEnvsCmd::List) => RequestPlan::get("/api/todo/envs"),
        TodoCmd::Envs(TodoEnvsCmd::Put { json }) => {
            RequestPlan::post("/api/todo/envs").with_opt_body(parse_body(Some(json.as_str()))?)
        }
        TodoCmd::Envs(TodoEnvsCmd::Get { name }) => {
            RequestPlan::get(format!("/api/todo/envs/{name}"))
        }
        TodoCmd::Envs(TodoEnvsCmd::Update { name, json }) => {
            RequestPlan::put(format!("/api/todo/envs/{name}"))
                .with_opt_body(parse_body(Some(json.as_str()))?)
        }
        TodoCmd::Envs(TodoEnvsCmd::Delete { name }) => {
            RequestPlan::delete(format!("/api/todo/envs/{name}"))
        }
        TodoCmd::Tools(TodoToolsCmd::List) => RequestPlan::get("/api/todo/tools"),
        TodoCmd::Tools(TodoToolsCmd::Import { json }) => {
            RequestPlan::post("/api/todo/tools/import")
                .with_opt_body(parse_body(Some(json.as_str()))?)
        }
        TodoCmd::Templates(TodoTemplatesCmd::List) => RequestPlan::get("/api/todo/templates"),
        TodoCmd::Templates(TodoTemplatesCmd::Put { json }) => {
            RequestPlan::post("/api/todo/templates").with_opt_body(parse_body(Some(json.as_str()))?)
        }
        TodoCmd::Templates(TodoTemplatesCmd::Get { name }) => {
            RequestPlan::get(format!("/api/todo/templates/{name}"))
        }
        TodoCmd::Templates(TodoTemplatesCmd::Delete { name }) => {
            RequestPlan::delete(format!("/api/todo/templates/{name}"))
        }
        TodoCmd::Templates(TodoTemplatesCmd::GetMeta { name }) => {
            RequestPlan::get(format!("/api/todo/templates/{name}/todo.json"))
        }
        TodoCmd::Templates(TodoTemplatesCmd::PutMeta { name, json }) => {
            RequestPlan::put(format!("/api/todo/templates/{name}/todo.json"))
                .with_opt_body(parse_body(Some(json.as_str()))?)
        }
        TodoCmd::Templates(TodoTemplatesCmd::NewVersion { name, json }) => {
            RequestPlan::post(format!("/api/todo/templates/{name}/new-version"))
                .with_opt_body(parse_body(Some(json.as_str()))?)
        }
        TodoCmd::Templates(TodoTemplatesCmd::GetContext { name, version }) => {
            RequestPlan::get(format!("/api/todo/templates/{name}/{version}/context.json"))
        }
        TodoCmd::Templates(TodoTemplatesCmd::PutContext {
            name,
            version,
            json,
        }) => RequestPlan::put(format!("/api/todo/templates/{name}/{version}/context.json"))
            .with_opt_body(parse_body(Some(json.as_str()))?),
        TodoCmd::Templates(TodoTemplatesCmd::GetBinding { name, version }) => {
            RequestPlan::get(format!("/api/todo/templates/{name}/{version}/env.json"))
        }
        TodoCmd::Templates(TodoTemplatesCmd::PutBinding {
            name,
            version,
            json,
        }) => RequestPlan::put(format!("/api/todo/templates/{name}/{version}/env.json"))
            .with_opt_body(parse_body(Some(json.as_str()))?),
        TodoCmd::Templates(TodoTemplatesCmd::DeleteVersion { name, version }) => {
            RequestPlan::delete(format!("/api/todo/templates/{name}/{version}"))
        }
        // The run handler requires a JSON body; an omitted --json degrades
        // to `{}` (the server then mints the workflow id).
        TodoCmd::Run {
            name,
            version,
            json,
        } => RequestPlan::post(format!("/api/todo/templates/{name}/{version}/run"))
            .with_body(parse_body(json.as_deref())?.unwrap_or_else(|| serde_json::json!({}))),
        TodoCmd::Workflows(TodoWorkflowsCmd::List) => RequestPlan::get("/api/todo/workflows"),
        TodoCmd::Workflows(TodoWorkflowsCmd::Get { id }) => {
            RequestPlan::get(format!("/api/todo/workflows/{id}"))
        }
        TodoCmd::Workflows(TodoWorkflowsCmd::Interrupt { id }) => {
            RequestPlan::post(format!("/api/todo/workflows/{id}/interrupt"))
        }
        TodoCmd::Workflows(TodoWorkflowsCmd::Resume { id }) => {
            RequestPlan::post(format!("/api/todo/workflows/{id}/resume"))
        }
        TodoCmd::Workflows(TodoWorkflowsCmd::Events { id, after }) => {
            RequestPlan::get(format!("/api/todo/workflows/{id}/events"))
                .with("after", after.to_string())
        }
    })
}

/// Execute a plan; `workflows events` streams SSE frames instead of
/// buffering.
pub async fn run(ctx: &Ctx, sub: TodoCmd) -> Result<i32> {
    let request = plan(&sub)?;
    if matches!(sub, TodoCmd::Workflows(TodoWorkflowsCmd::Events { .. })) {
        return exec_stream(ctx, request).await;
    }
    exec_plan(ctx, request).await
}
