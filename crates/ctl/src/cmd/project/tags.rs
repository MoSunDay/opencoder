use crate::http::{urlencode, RequestPlan};
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum TagsCmd {
    List {
        #[arg(long)]
        scope_type: Option<String>,
        #[arg(long)]
        scope_id: Option<String>,
    },
    Create {
        #[arg(long)]
        json: String,
    },
    Patch {
        id: String,
        #[arg(long)]
        json: String,
    },
    Delete {
        id: String,
    },
}

pub fn plan(sub: &TagsCmd) -> Result<RequestPlan> {
    Ok(match sub {
        TagsCmd::List {
            scope_type,
            scope_id,
        } => RequestPlan::get("/api/project/tags")
            .with_opt("scope_type", scope_type.clone())
            .with_opt("scope_id", scope_id.clone()),
        TagsCmd::Create { json } => {
            RequestPlan::post("/api/project/tags").with_body(super::required_body(json)?)
        }
        TagsCmd::Patch { id, json } => {
            RequestPlan::patch(format!("/api/project/tags/{}", urlencode(id)))
                .with_body(super::required_body(json)?)
        }
        TagsCmd::Delete { id } => {
            RequestPlan::delete(format!("/api/project/tags/{}", urlencode(id)))
        }
    })
}
