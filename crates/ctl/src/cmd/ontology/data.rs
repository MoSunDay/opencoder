use super::{body, segment};
use crate::http::RequestPlan;
use anyhow::Result;
use clap::Subcommand;

/// Soft deletion is an update with is_deleted and expected_revision.
#[derive(Subcommand, Debug)]
pub enum CollectionCmd {
    List {
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
pub enum NestedCmd {
    List {
        type_id: String,
        #[arg(long)]
        include_deleted: bool,
    },
    Create {
        type_id: String,
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
pub enum EntityCmd {
    List {
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 100)]
        limit: u32,
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
    SetValue {
        id: String,
        attribute: String,
        #[arg(long)]
        json: String,
    },
    SetText {
        id: String,
        attribute: String,
        #[arg(long)]
        json: String,
    },
    SetNfsPath {
        id: String,
        attribute: String,
        #[arg(long)]
        json: String,
    },
    TextHistory {
        id: String,
        attribute: String,
    },
    TextGet {
        id: String,
        attribute: String,
        revision: i64,
    },
}

pub(super) fn collection(path: &str, cmd: &CollectionCmd) -> Result<RequestPlan> {
    Ok(match cmd {
        CollectionCmd::List { include_deleted } => {
            RequestPlan::get(path).with("include_deleted", include_deleted.to_string())
        }
        CollectionCmd::Create { json } => RequestPlan::post(path).with_body(body(json)?),
        CollectionCmd::Update { id, json } => {
            RequestPlan::patch(format!("{path}/{}", segment(id))).with_body(body(json)?)
        }
    })
}

pub(super) fn nested(root: &str, name: &str, cmd: &NestedCmd) -> Result<RequestPlan> {
    Ok(match cmd {
        NestedCmd::List {
            type_id,
            include_deleted,
        } => RequestPlan::get(format!("{root}/entity-types/{}/{name}", segment(type_id)))
            .with("include_deleted", include_deleted.to_string()),
        NestedCmd::Create { type_id, json } => {
            RequestPlan::post(format!("{root}/entity-types/{}/{name}", segment(type_id)))
                .with_body(body(json)?)
        }
        NestedCmd::Update { id, json } => {
            RequestPlan::patch(format!("{root}/{name}/{}", segment(id))).with_body(body(json)?)
        }
    })
}

pub(super) fn entity(root: &str, cmd: &EntityCmd) -> Result<RequestPlan> {
    let root = format!("{root}/entities");
    let attr = |id: &str, attribute: &str| {
        format!("{root}/{}/attributes/{}", segment(id), segment(attribute))
    };
    Ok(match cmd {
        EntityCmd::List {
            offset,
            limit,
            include_deleted,
        } => RequestPlan::get(root)
            .with("offset", offset.to_string())
            .with("limit", limit.to_string())
            .with("include_deleted", include_deleted.to_string()),
        EntityCmd::Get { id } => RequestPlan::get(format!("{root}/{}", segment(id))),
        EntityCmd::Create { json } => RequestPlan::post(root).with_body(body(json)?),
        EntityCmd::Update { id, json } => {
            RequestPlan::patch(format!("{root}/{}", segment(id))).with_body(body(json)?)
        }
        EntityCmd::SetValue {
            id,
            attribute,
            json,
        } => RequestPlan::put(attr(id, attribute)).with_body(body(json)?),
        EntityCmd::SetText {
            id,
            attribute,
            json,
        } => RequestPlan::put(format!("{}/text", attr(id, attribute))).with_body(body(json)?),
        EntityCmd::SetNfsPath {
            id,
            attribute,
            json,
        } => RequestPlan::put(format!("{}/nfs-path", attr(id, attribute))).with_body(body(json)?),
        EntityCmd::TextHistory { id, attribute } => {
            RequestPlan::get(format!("{}/text", attr(id, attribute)))
        }
        EntityCmd::TextGet {
            id,
            attribute,
            revision,
        } => RequestPlan::get(format!("{}/text/{revision}", attr(id, attribute))),
    })
}
