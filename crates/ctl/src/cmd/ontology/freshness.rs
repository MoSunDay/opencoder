//! Freshness commands under `/api/ontology/envs/:env/freshness`: policies
//! define refresh executors, runs record refresh attempts and reviews record
//! verdicts. Commands map to routes only; they never execute executors.

use super::{body, segment};
use crate::http::RequestPlan;
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum FreshnessCmd {
    #[command(subcommand)]
    Policies(FreshnessPoliciesCmd),
    #[command(subcommand)]
    Runs(FreshnessRunsCmd),
    #[command(subcommand)]
    Reviews(FreshnessReviewsCmd),
}

/// Policy definitions only; `update` with is_deleted and expected_revision
/// performs the soft deletion, matching the collection command semantics.
#[derive(Subcommand, Debug)]
pub enum FreshnessPoliciesCmd {
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
}

#[derive(Subcommand, Debug)]
pub enum FreshnessRunsCmd {
    List {
        #[arg(long)]
        policy_id: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long)]
        include_deleted: bool,
    },
    Get {
        id: String,
    },
    /// Ask the control plane to enqueue one run; the server answers 202.
    Trigger {
        policy_id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum FreshnessReviewsCmd {
    List {
        #[arg(long)]
        run_id: Option<String>,
    },
    Create {
        #[arg(long)]
        json: String,
    },
}

pub(super) fn plan(root: &str, cmd: &FreshnessCmd) -> Result<RequestPlan> {
    Ok(match cmd {
        FreshnessCmd::Policies(c) => policies(root, c)?,
        FreshnessCmd::Runs(c) => runs(root, c),
        FreshnessCmd::Reviews(c) => reviews(root, c)?,
    })
}

fn policies(root: &str, cmd: &FreshnessPoliciesCmd) -> Result<RequestPlan> {
    let path = format!("{root}/policies");
    Ok(match cmd {
        FreshnessPoliciesCmd::List { include_deleted } => {
            RequestPlan::get(path).with("include_deleted", include_deleted.to_string())
        }
        FreshnessPoliciesCmd::Get { id } => RequestPlan::get(format!("{path}/{}", segment(id))),
        FreshnessPoliciesCmd::Create { json } => RequestPlan::post(path).with_body(body(json)?),
        FreshnessPoliciesCmd::Update { id, json } => {
            RequestPlan::patch(format!("{path}/{}", segment(id))).with_body(body(json)?)
        }
    })
}

fn runs(root: &str, cmd: &FreshnessRunsCmd) -> RequestPlan {
    let path = format!("{root}/runs");
    match cmd {
        FreshnessRunsCmd::List {
            policy_id,
            offset,
            limit,
            include_deleted,
        } => RequestPlan::get(path)
            .with_opt("policy_id", policy_id.clone())
            .with("offset", offset.to_string())
            .with("limit", limit.to_string())
            .with("include_deleted", include_deleted.to_string()),
        FreshnessRunsCmd::Get { id } => RequestPlan::get(format!("{path}/{}", segment(id))),
        FreshnessRunsCmd::Trigger { policy_id } => {
            RequestPlan::post(format!("{root}/policies/{}/runs", segment(policy_id)))
        }
    }
}

fn reviews(root: &str, cmd: &FreshnessReviewsCmd) -> Result<RequestPlan> {
    let path = format!("{root}/reviews");
    Ok(match cmd {
        FreshnessReviewsCmd::List { run_id } => {
            RequestPlan::get(path).with_opt("run_id", run_id.clone())
        }
        FreshnessReviewsCmd::Create { json } => RequestPlan::post(path).with_body(body(json)?),
    })
}
