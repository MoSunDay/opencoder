use super::{body, segment};
use crate::http::RequestPlan;
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum DirectoryCmd {
    Tree {
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
    Move {
        id: String,
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum VectorCmd {
    Get {
        id: String,
    },
    Set {
        entity: String,
        attribute: String,
        #[arg(long)]
        json: String,
    },
    Search {
        #[arg(long)]
        json: String,
    },
}

#[derive(Args, Debug)]
pub struct GraphArgs {
    #[arg(long)]
    center: Vec<String>,
    #[arg(long)]
    entity_type_id: Vec<String>,
    #[arg(long)]
    relationship_type_id: Vec<String>,
    #[arg(long)]
    depth: Option<u8>,
    #[arg(long)]
    upstream_depth: Option<u8>,
    #[arg(long)]
    downstream_depth: Option<u8>,
    #[arg(long)]
    pinned_only: bool,
    #[arg(long)]
    expand_neighbors: bool,
}

pub(super) fn graph(root: &str, cmd: &GraphArgs) -> RequestPlan {
    let mut request = RequestPlan::get(format!("{root}/graph"))
        .with_opt("depth", cmd.depth.map(|v| v.to_string()))
        .with_opt("upstream_depth", cmd.upstream_depth.map(|v| v.to_string()))
        .with_opt(
            "downstream_depth",
            cmd.downstream_depth.map(|v| v.to_string()),
        )
        .with("pinned_only", cmd.pinned_only.to_string())
        .with("expand_neighbors", cmd.expand_neighbors.to_string());
    for (key, values) in [
        ("center", &cmd.center),
        ("entity_type_id", &cmd.entity_type_id),
        ("relationship_type_id", &cmd.relationship_type_id),
    ] {
        for value in values {
            request = request.with(key, value.clone());
        }
    }
    request
}

pub(super) fn directory(root: &str, cmd: &DirectoryCmd) -> Result<RequestPlan> {
    let root = format!("{root}/directories");
    Ok(match cmd {
        DirectoryCmd::Tree { include_deleted } => RequestPlan::get(format!("{root}/tree"))
            .with("include_deleted", include_deleted.to_string()),
        DirectoryCmd::Create { json } => {
            RequestPlan::post(format!("{root}/tree")).with_body(body(json)?)
        }
        DirectoryCmd::Update { id, json } => {
            RequestPlan::patch(format!("{root}/{}", segment(id))).with_body(body(json)?)
        }
        DirectoryCmd::Move { id, json } => {
            RequestPlan::patch(format!("{root}/{}/move", segment(id))).with_body(body(json)?)
        }
    })
}

pub(super) fn vector(root: &str, cmd: &VectorCmd) -> Result<RequestPlan> {
    Ok(match cmd {
        VectorCmd::Get { id } => RequestPlan::get(format!("{root}/vectors/{}", segment(id))),
        VectorCmd::Set {
            entity,
            attribute,
            json,
        } => RequestPlan::put(format!(
            "{root}/entities/{}/vectors/{}",
            segment(entity),
            segment(attribute)
        ))
        .with_body(body(json)?),
        VectorCmd::Search { json } => {
            RequestPlan::post(format!("{root}/vector-search")).with_body(body(json)?)
        }
    })
}
