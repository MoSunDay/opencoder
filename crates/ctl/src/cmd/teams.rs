//! `teams` domain: execution team definitions (`/api/teams`). Pure `plan()`
//! mapping only; execution goes through the shared `exec_plan` transport.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::{exec_plan, raw::parse_body};
use crate::ctx::Ctx;
use crate::http::RequestPlan;

#[derive(Subcommand, Debug)]
pub enum TeamsCmd {
    /// List team definitions (the retired `system` team is filtered out).
    #[command(alias = "ls")]
    List,
    /// Create/replace a team (body: a TeamDefinition JSON document).
    Put {
        /// Team body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
}

/// Pure mapping: subcommand → request plan. The list handler takes no
/// query extractor; the save handler requires a JSON body.
pub fn plan(sub: &TeamsCmd) -> Result<RequestPlan> {
    Ok(match sub {
        TeamsCmd::List => RequestPlan::get("/api/teams"),
        TeamsCmd::Put { json } => {
            RequestPlan::post("/api/teams").with_opt_body(parse_body(Some(json.as_str()))?)
        }
    })
}

pub async fn run(ctx: &Ctx, sub: TeamsCmd) -> Result<i32> {
    exec_plan(ctx, plan(&sub)?).await
}
