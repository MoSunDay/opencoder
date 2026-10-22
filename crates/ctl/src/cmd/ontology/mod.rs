//! Ontology commands use the same routes and revisions as the web client.
mod data;
mod graph;

use crate::{
    cmd::{exec_plan, raw::parse_body},
    ctx::Ctx,
    http::RequestPlan,
    out,
};
use anyhow::Result;
use clap::{Args, Subcommand};
pub use data::{CollectionCmd, EntityCmd, NestedCmd};
pub use graph::{DirectoryCmd, GraphArgs, VectorCmd};

#[derive(Args, Debug)]
pub struct OntologyCmd {
    /// Ontology data space on the selected server.
    #[arg(long, global = true, default_value = "debug")]
    pub env: String,
    #[command(subcommand)]
    pub command: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Current identity and Ontology permissions.
    Session,
    #[command(subcommand)]
    Environments(CollectionCmd),
    #[command(subcommand)]
    EntityTypes(CollectionCmd),
    #[command(subcommand)]
    RelationshipTypes(CollectionCmd),
    #[command(subcommand)]
    Attributes(NestedCmd),
    /// Action definitions only; these commands do not execute actions.
    #[command(subcommand)]
    Actions(NestedCmd),
    #[command(subcommand)]
    Entities(EntityCmd),
    #[command(subcommand)]
    Relationships(RelationshipCmd),
    #[command(subcommand)]
    Directories(DirectoryCmd),
    #[command(subcommand)]
    Aspects(AspectCmd),
    Graph(GraphArgs),
    #[command(subcommand)]
    Vectors(VectorCmd),
    #[command(subcommand)]
    Capabilities(CapabilityCmd),
    #[command(subcommand)]
    Sync(SyncCmd),
    #[command(subcommand)]
    Nfs(NfsCmd),
}

#[derive(Subcommand, Debug)]
pub enum RelationshipCmd {
    List {
        #[arg(long)]
        anchor: Option<String>,
        #[arg(long)]
        include_deleted: bool,
    },
    Create {
        #[arg(long)]
        json: String,
    },
    Update {
        id: String,
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum AspectCmd {
    List {
        #[arg(long)]
        include_deleted: bool,
    },
    Get {
        id: String,
    },
    Create {
        #[arg(long)]
        json: String,
    },
    Update {
        id: String,
        #[arg(long)]
        json: String,
    },
    Delete {
        id: String,
        #[arg(long)]
        expected_revision: i64,
    },
}

#[derive(Subcommand, Debug)]
pub enum CapabilityCmd {
    List {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        include_deleted: bool,
    },
    /// Read a capability by its Ontology entity ID.
    Get { id: String },
}

#[derive(Subcommand, Debug)]
pub enum SyncCmd {
    Status,
    /// Read-only comparison; exits 4 when synchronization is needed.
    Check,
    /// Wait for one complete synchronization and return its changes.
    Run,
}

#[derive(Subcommand, Debug)]
pub enum NfsCmd {
    Status,
    Set {
        #[arg(long)]
        json: String,
    },
}

pub(super) fn segment(value: &str) -> String {
    crate::http::urlencode(value)
}

pub(super) fn body(raw: &str) -> Result<serde_json::Value> {
    parse_body(Some(raw))?.ok_or_else(|| anyhow::anyhow!("missing JSON body"))
}

pub fn plan(cmd: &OntologyCmd) -> Result<RequestPlan> {
    let root = format!("/api/ontology/envs/{}", segment(&cmd.env));
    Ok(match &cmd.command {
        Action::Session => RequestPlan::get("/api/ontology/session"),
        Action::Environments(c) => data::collection("/api/ontology/environments", c)?,
        Action::EntityTypes(c) => data::collection(&format!("{root}/entity-types"), c)?,
        Action::RelationshipTypes(c) => data::collection(&format!("{root}/relationship-types"), c)?,
        Action::Attributes(c) => data::nested(&root, "attributes", c)?,
        Action::Actions(c) => data::nested(&root, "actions", c)?,
        Action::Entities(c) => data::entity(&root, c)?,
        Action::Directories(c) => graph::directory(&root, c)?,
        Action::Vectors(c) => graph::vector(&root, c)?,
        Action::Graph(c) => graph::graph(&root, c),
        Action::Relationships(c) => {
            let path = format!("{root}/relationships");
            match c {
                RelationshipCmd::List {
                    anchor,
                    include_deleted,
                } => RequestPlan::get(path)
                    .with_opt("anchor", anchor.clone())
                    .with("include_deleted", include_deleted.to_string()),
                RelationshipCmd::Create { json } => RequestPlan::post(path).with_body(body(json)?),
                RelationshipCmd::Update { id, json } => {
                    RequestPlan::patch(format!("{path}/{}", segment(id))).with_body(body(json)?)
                }
            }
        }
        Action::Aspects(c) => {
            let path = format!("{root}/graph-aspects");
            match c {
                AspectCmd::List { include_deleted } => {
                    RequestPlan::get(path).with("include_deleted", include_deleted.to_string())
                }
                AspectCmd::Get { id } => RequestPlan::get(format!("{path}/{}", segment(id))),
                AspectCmd::Create { json } => RequestPlan::post(path).with_body(body(json)?),
                AspectCmd::Update { id, json } => {
                    RequestPlan::patch(format!("{path}/{}", segment(id))).with_body(body(json)?)
                }
                AspectCmd::Delete {
                    id,
                    expected_revision,
                } => RequestPlan::delete(format!("{path}/{}", segment(id)))
                    .with("expected_revision", expected_revision.to_string()),
            }
        }
        Action::Capabilities(c) => match c {
            CapabilityCmd::List {
                kind,
                query,
                offset,
                limit,
                include_deleted,
            } => RequestPlan::get(format!("{root}/capabilities"))
                .with_opt("kind", kind.clone())
                .with_opt("query", query.clone())
                .with("offset", offset.to_string())
                .with("limit", limit.to_string())
                .with("include_deleted", include_deleted.to_string()),
            CapabilityCmd::Get { id } => {
                RequestPlan::get(format!("{root}/capabilities/{}", segment(id)))
            }
        },
        Action::Sync(SyncCmd::Status) => RequestPlan::get(format!("{root}/sync")),
        Action::Sync(SyncCmd::Check) => RequestPlan::get(format!("{root}/sync/check")),
        Action::Sync(SyncCmd::Run) => RequestPlan::post(format!("{root}/sync")),
        Action::Nfs(NfsCmd::Status) => RequestPlan::get("/api/ontology/nfs"),
        Action::Nfs(NfsCmd::Set { json }) => {
            RequestPlan::post("/api/ontology/nfs").with_body(body(json)?)
        }
    })
}

pub async fn run(ctx: &Ctx, cmd: OntologyCmd) -> Result<i32> {
    let request = plan(&cmd)?;
    if !matches!(cmd.command, Action::Sync(SyncCmd::Check)) {
        return exec_plan(ctx, request).await;
    }
    let outcome = match crate::http::send(ctx, &request).await {
        Ok(outcome) => outcome,
        Err(error) => {
            out::fail_transport(&format!("{error:#}"));
            return Ok(1);
        }
    };
    if !outcome.is_success() {
        out::fail(outcome.status, &outcome.error_message());
        return Ok(crate::http::exit_code(outcome.status));
    }
    let value = outcome
        .json
        .ok_or_else(|| anyhow::anyhow!("invalid synchronization response"))?;
    out::json(&value);
    Ok(if value["in_sync"] == true { 0 } else { 4 })
}
