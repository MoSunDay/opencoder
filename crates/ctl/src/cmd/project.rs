//! `project` domain: user-curated goal → milestone → todo tracking with
//! per-todo plan/execute runs. Pure `plan()` mapping only; execution goes
//! through the shared `exec_plan` transport.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::exec_plan;
use crate::cmd::raw::parse_body;
use crate::ctx::Ctx;
use crate::http::RequestPlan;

/// Parse a required `--json` value (inline JSON or `@file`). A required
/// flag that somehow yields no body is a caller bug, not a silent skip.
fn required_body(raw: &str) -> Result<serde_json::Value> {
    parse_body(Some(raw))?.ok_or_else(|| anyhow::anyhow!("required --json body is empty"))
}

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// GET /api/project/overview — nested goals→milestones→todos snapshot.
    Overview,
    /// Goal records.
    #[command(subcommand)]
    Goals(GoalsCmd),
    /// Milestones grouped under goals.
    #[command(subcommand)]
    Milestones(MilestonesCmd),
    /// Todo lifecycle, plan/execute dispatch and run history.
    #[command(subcommand)]
    Todos(TodosCmd),
}

#[derive(Subcommand, Debug)]
pub enum GoalsCmd {
    /// GET /api/project/goals — all goals, sort then created_at order.
    List,
    /// POST /api/project/goals — {"title","detail_md"?,"sort"?}.
    Create {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// PATCH /api/project/goals/{id} — partial record update.
    Patch {
        id: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/project/goals/{id}.
    Delete { id: String },
}

#[derive(Subcommand, Debug)]
pub enum MilestonesCmd {
    /// GET /api/project/milestones — one goal's milestones via --goal,
    /// or across all goals when absent.
    List {
        /// Filter by goal id (goal_id query param).
        #[arg(long)]
        goal: Option<String>,
    },
    /// POST /api/project/milestones — {"goal_id","title","detail_md"?,"sort"?}.
    Create {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// PATCH /api/project/milestones/{id}.
    Patch {
        id: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/project/milestones/{id}.
    Delete { id: String },
}

#[derive(Subcommand, Debug)]
pub enum TodosCmd {
    /// GET /api/project/todos — one milestone's todos via --milestone,
    /// or every todo (backlog included) when absent.
    List {
        /// Filter by milestone id (milestone_id query param).
        #[arg(long)]
        milestone: Option<String>,
    },
    /// POST /api/project/todos — {"title","draft","milestone_id"?,"agent"?,
    /// "executor_kind"?,"executor_ref"?,"executor_spec"?}.
    Create {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// PATCH /api/project/todos/{id}.
    Patch {
        id: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/project/todos/{id}.
    Delete { id: String },
    /// POST /api/project/todos/{id}/plan — spawn a plan run (202 + run_id).
    Plan {
        id: String,
        /// Optional input object; may carry "run_id" to resume a prior run.
        #[arg(long)]
        json: Option<String>,
    },
    /// POST /api/project/todos/{id}/execute — drive the todo's current
    /// plan in a new-or-resumed session (202 + run_id).
    Execute {
        id: String,
        /// Optional input object; may carry "run_id" to resume a prior run.
        #[arg(long)]
        json: Option<String>,
    },
    /// GET /api/project/todos/{id}/runs — run history, newest version
    /// first (20 per page).
    Runs {
        id: String,
        /// Page cursor: only runs with version < this value.
        #[arg(long)]
        before_version: Option<i64>,
    },
    /// POST /api/project/runs/{id}/cancel — idempotent cancel of a run.
    CancelRun { id: String },
}

pub fn plan(sub: &ProjectCmd) -> Result<RequestPlan> {
    Ok(match sub {
        ProjectCmd::Overview => RequestPlan::get("/api/project/overview"),
        ProjectCmd::Goals(sub) => plan_goals(sub)?,
        ProjectCmd::Milestones(sub) => plan_milestones(sub)?,
        ProjectCmd::Todos(sub) => plan_todos(sub)?,
    })
}

fn plan_goals(sub: &GoalsCmd) -> Result<RequestPlan> {
    Ok(match sub {
        GoalsCmd::List => RequestPlan::get("/api/project/goals"),
        GoalsCmd::Create { json } => {
            RequestPlan::post("/api/project/goals").with_body(required_body(json)?)
        }
        GoalsCmd::Patch { id, json } => {
            RequestPlan::patch(format!("/api/project/goals/{id}")).with_body(required_body(json)?)
        }
        GoalsCmd::Delete { id } => RequestPlan::delete(format!("/api/project/goals/{id}")),
    })
}

fn plan_milestones(sub: &MilestonesCmd) -> Result<RequestPlan> {
    Ok(match sub {
        MilestonesCmd::List { goal } => {
            RequestPlan::get("/api/project/milestones").with_opt("goal_id", goal.clone())
        }
        MilestonesCmd::Create { json } => {
            RequestPlan::post("/api/project/milestones").with_body(required_body(json)?)
        }
        MilestonesCmd::Patch { id, json } => {
            RequestPlan::patch(format!("/api/project/milestones/{id}"))
                .with_body(required_body(json)?)
        }
        MilestonesCmd::Delete { id } => {
            RequestPlan::delete(format!("/api/project/milestones/{id}"))
        }
    })
}

fn plan_todos(sub: &TodosCmd) -> Result<RequestPlan> {
    Ok(match sub {
        TodosCmd::List { milestone } => {
            RequestPlan::get("/api/project/todos").with_opt("milestone_id", milestone.clone())
        }
        TodosCmd::Create { json } => {
            RequestPlan::post("/api/project/todos").with_body(required_body(json)?)
        }
        TodosCmd::Patch { id, json } => {
            RequestPlan::patch(format!("/api/project/todos/{id}")).with_body(required_body(json)?)
        }
        TodosCmd::Delete { id } => RequestPlan::delete(format!("/api/project/todos/{id}")),
        TodosCmd::Plan { id, json } => RequestPlan::post(format!("/api/project/todos/{id}/plan"))
            .with_opt_body(parse_body(json.as_deref())?),
        TodosCmd::Execute { id, json } => {
            RequestPlan::post(format!("/api/project/todos/{id}/execute"))
                .with_opt_body(parse_body(json.as_deref())?)
        }
        TodosCmd::Runs { id, before_version } => {
            RequestPlan::get(format!("/api/project/todos/{id}/runs")).with_opt(
                "before_version",
                before_version.map(|version| version.to_string()),
            )
        }
        TodosCmd::CancelRun { id } => RequestPlan::post(format!("/api/project/runs/{id}/cancel")),
    })
}

pub async fn run(ctx: &Ctx, sub: ProjectCmd) -> Result<i32> {
    exec_plan(ctx, plan(&sub)?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn required_body_accepts_inline_and_at_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("goal.json");
        std::fs::write(&file, r#"{"title":"ship"}"#).unwrap();
        assert_eq!(
            required_body(r#"{"title":"x"}"#).unwrap(),
            json!({"title": "x"})
        );
        assert_eq!(
            required_body(format!("@{}", file.display()).as_str()).unwrap(),
            json!({"title": "ship"})
        );
        assert!(required_body("nope").is_err());
    }

    #[test]
    fn todo_run_routes_map() {
        let plan = plan_todos(&TodosCmd::Runs {
            id: "t1".into(),
            before_version: Some(7),
        })
        .unwrap();
        assert_eq!(plan.path, "/api/project/todos/t1/runs");
        assert_eq!(
            plan.query,
            vec![("before_version".to_owned(), "7".to_owned())]
        );

        let plan = plan_todos(&TodosCmd::CancelRun { id: "r9".into() }).unwrap();
        assert_eq!(plan.method, reqwest::Method::POST);
        assert_eq!(plan.path, "/api/project/runs/r9/cancel");
        assert!(plan.body.is_none());
    }
}
