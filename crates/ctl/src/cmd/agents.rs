//! `agents` domain: versioned custom agents — reference cards, the active
//! marker, versioned resource pools (prompts/skills/tools/memory) and the
//! NFS read-only export. Pure `plan()` mapping only; execution goes through
//! the shared `exec_plan` transport.

use anyhow::{bail, Result};
use clap::Subcommand;

use crate::cmd::exec_plan;
use crate::cmd::raw::parse_body;
use crate::ctx::Ctx;
use crate::http::RequestPlan;

/// Parse a required `--json` value (inline JSON or `@file`).
fn required_body(raw: &str) -> Result<serde_json::Value> {
    parse_body(Some(raw))?.ok_or_else(|| anyhow::anyhow!("required --json body is empty"))
}

#[derive(Subcommand, Debug)]
pub enum AgentCmd {
    /// GET /api/agents — every card plus the active marker.
    List,
    /// POST /api/agents — {"name","current"?} new reference card.
    Create {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// PUT /api/agents/{name} — {"current":{...}} reference rewrite.
    Update {
        name: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/agents/{name}.
    Delete { name: String },
    /// PATCH /api/agents/active — {"active":name|null}.
    Active {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// GET /api/agents/{name}/meta — full card, history included.
    Meta { name: String },
    /// Versioned resource pools.
    #[command(subcommand)]
    Resources(AgentResourcesCmd),
    /// NFS read-only export lifecycle.
    #[command(subcommand)]
    Nfs(NfsCmd),
}

#[derive(Subcommand, Debug)]
pub enum AgentResourcesCmd {
    /// GET /api/agents/resources/{cat} — pool entries with version history.
    List {
        /// Category: prompts | skills | tools | memory.
        cat: String,
    },
    /// POST /api/agents/resources/{cat} — {"name","files":[{path,
    /// content_b64}]} creates the resource at v1.
    Create {
        /// Category: prompts | skills | tools | memory.
        cat: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// PUT /api/agents/resources/{cat}/{name} — new version (same body
    /// shape as create).
    Update {
        /// Category: prompts | skills | tools | memory.
        cat: String,
        name: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/agents/resources/{cat}/{name} — 409 while any card
    /// still references the pool.
    Delete {
        /// Category: prompts | skills | tools | memory.
        cat: String,
        name: String,
    },
    /// GET /api/agents/resources/{cat}/{name}/meta — current pointer and
    /// version history.
    Meta {
        /// Category: prompts | skills | tools | memory.
        cat: String,
        name: String,
    },
    /// POST /api/agents/resources/{cat}/{name}/rollback — {"version":N}
    /// pointer switch, version dirs stay.
    Rollback {
        /// Category: prompts | skills | tools | memory.
        cat: String,
        name: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// GET /api/agents/resources/{cat}/{name}/versions/{v}/files/{path} —
    /// one file's content; PATH takes multiple slash segments.
    File {
        /// Category: prompts | skills | tools | memory.
        cat: String,
        name: String,
        /// Version number (plain integer, e.g. 2 for v2).
        version: u32,
        /// File path inside the version dir, e.g. `guides setup.md`
        /// or `python helpers io.py` for `python/helpers/io.py`.
        #[arg(value_name = "PATH", num_args = 1.., required = true)]
        path: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum NfsCmd {
    /// GET /api/agents/nfs — status snapshot (stopped defaults when off).
    Status,
    /// POST /api/agents/nfs — {"enabled":bool} lifecycle switch
    /// (idempotent both ways).
    Set {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
}

pub fn plan(sub: &AgentCmd) -> Result<RequestPlan> {
    Ok(match sub {
        AgentCmd::List => RequestPlan::get("/api/agents"),
        AgentCmd::Create { json } => {
            RequestPlan::post("/api/agents").with_body(required_body(json)?)
        }
        AgentCmd::Update { name, json } => {
            RequestPlan::put(format!("/api/agents/{name}")).with_body(required_body(json)?)
        }
        AgentCmd::Delete { name } => RequestPlan::delete(format!("/api/agents/{name}")),
        AgentCmd::Active { json } => {
            RequestPlan::patch("/api/agents/active").with_body(required_body(json)?)
        }
        AgentCmd::Meta { name } => RequestPlan::get(format!("/api/agents/{name}/meta")),
        AgentCmd::Resources(sub) => plan_resources(sub)?,
        AgentCmd::Nfs(sub) => plan_nfs(sub)?,
    })
}

/// Join positional path segments into the single wildcard segment the axum
/// route captures (`a b c` → `a/b/c`). Empty path is rejected here so the
/// pure planner never emits a dangling `/files/` URL.
fn join_path(segments: &[String]) -> Result<String> {
    if segments.is_empty() {
        bail!("resource file path must have at least one segment");
    }
    Ok(segments.join("/"))
}

fn plan_resources(sub: &AgentResourcesCmd) -> Result<RequestPlan> {
    Ok(match sub {
        AgentResourcesCmd::List { cat } => RequestPlan::get(format!("/api/agents/resources/{cat}")),
        AgentResourcesCmd::Create { cat, json } => {
            RequestPlan::post(format!("/api/agents/resources/{cat}"))
                .with_body(required_body(json)?)
        }
        AgentResourcesCmd::Update { cat, name, json } => {
            RequestPlan::put(format!("/api/agents/resources/{cat}/{name}"))
                .with_body(required_body(json)?)
        }
        AgentResourcesCmd::Delete { cat, name } => {
            RequestPlan::delete(format!("/api/agents/resources/{cat}/{name}"))
        }
        AgentResourcesCmd::Meta { cat, name } => {
            RequestPlan::get(format!("/api/agents/resources/{cat}/{name}/meta"))
        }
        AgentResourcesCmd::Rollback { cat, name, json } => {
            RequestPlan::post(format!("/api/agents/resources/{cat}/{name}/rollback"))
                .with_body(required_body(json)?)
        }
        AgentResourcesCmd::File {
            cat,
            name,
            version,
            path,
        } => RequestPlan::get(format!(
            "/api/agents/resources/{cat}/{name}/versions/{version}/files/{}",
            join_path(path)?
        )),
    })
}

fn plan_nfs(sub: &NfsCmd) -> Result<RequestPlan> {
    Ok(match sub {
        NfsCmd::Status => RequestPlan::get("/api/agents/nfs"),
        NfsCmd::Set { json } => {
            RequestPlan::post("/api/agents/nfs").with_body(required_body(json)?)
        }
    })
}

pub async fn run(ctx: &Ctx, sub: AgentCmd) -> Result<i32> {
    exec_plan(ctx, plan(&sub)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_routes_map_methods() {
        assert_eq!(
            plan(&AgentCmd::List).unwrap(),
            RequestPlan::get("/api/agents")
        );
        assert_eq!(
            plan(&AgentCmd::Meta {
                name: "reviewer".into()
            })
            .unwrap(),
            RequestPlan::get("/api/agents/reviewer/meta")
        );
        assert_eq!(
            plan(&AgentCmd::Delete {
                name: "reviewer".into()
            })
            .unwrap(),
            RequestPlan::delete("/api/agents/reviewer")
        );
        let active = plan(&AgentCmd::Active {
            json: r#"{"active":null}"#.into(),
        })
        .unwrap();
        assert_eq!(active.method, reqwest::Method::PATCH);
        assert_eq!(active.path, "/api/agents/active");
    }

    #[test]
    fn file_route_joins_wildcard_segments() {
        let planned = plan_resources(&AgentResourcesCmd::File {
            cat: "skills".into(),
            name: "shell".into(),
            version: 3,
            path: vec!["python".into(), "helpers".into(), "io.py".into()],
        })
        .unwrap();
        assert_eq!(
            planned.path,
            "/api/agents/resources/skills/shell/versions/3/files/python/helpers/io.py"
        );
        assert!(plan_resources(&AgentResourcesCmd::File {
            cat: "skills".into(),
            name: "shell".into(),
            version: 3,
            path: Vec::new(),
        })
        .is_err());
    }
}
