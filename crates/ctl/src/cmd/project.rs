//! `project` domain: user-curated goal → initiative → todo tracking with
//! per-todo plan/execute runs. Pure `plan()` mapping only; execution goes
//! through the shared `exec_plan` transport.

pub mod conversation;
pub mod tags;

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
    /// GET /api/project/overview — nested goals→initiatives→todos snapshot.
    Overview,
    /// Project and initiative tag definitions.
    #[command(subcommand)]
    Tags(tags::TagsCmd),
    /// Goal records.
    #[command(subcommand)]
    Goals(GoalsCmd),
    /// Initiatives grouped under goals.
    #[command(subcommand)]
    Initiatives(InitiativesCmd),
    /// Todo lifecycle, plan/execute dispatch and run history.
    #[command(subcommand)]
    Todos(TodosCmd),
}

#[derive(Subcommand, Debug)]
pub enum GoalsCmd {
    /// Fetch one record.
    Get { id: String },
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
pub enum InitiativesCmd {
    /// Fetch one record.
    Get { id: String },
    /// GET /api/project/initiatives — one goal's initiatives via --goal,
    /// or across all goals when absent.
    List {
        /// Filter by goal id (goal_id query param).
        #[arg(long)]
        goal: Option<String>,
    },
    /// POST /api/project/initiatives — {"goal_id","title","detail_md"?,"sort"?}.
    Create {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// PATCH /api/project/initiatives/{id}.
    Patch {
        id: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/project/initiatives/{id}.
    Delete { id: String },
}

#[derive(Subcommand, Debug)]
pub enum TodosCmd {
    /// Fetch a TODO including its selected tags and capability.
    Get { id: String },
    /// Move/reorder a board lane: {initiative_id,board_status,ids}.
    Reorder {
        #[arg(long)]
        json: String,
    },
    /// Continue or guide this TODO's Agent/Operator conversation.
    Prompt(conversation::Prompt),
    /// Read the linked conversation with native pagination.
    Messages(conversation::Messages),
    /// Follow native execution events, resuming after a sequence.
    Events(conversation::Events),
    /// Read the current result from its owning node.
    Result(conversation::Target),
    /// Interrupt the linked conversation.
    Interrupt(conversation::Target),
    /// GET /api/project/todos — one initiative's todos via --initiative,
    /// or every todo (backlog included) when absent.
    List {
        /// Filter by initiative id (initiative_id query param).
        #[arg(long)]
        initiative: Option<String>,
    },
    /// POST /api/project/todos — {"title","draft","initiative_id"?,"agent"?,
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
    /// List the capability/execution references attached to this TODO.
    Links { id: String },
    /// Attach an existing execution, including the operator's current session.
    Attach {
        id: String,
        execution_id: String,
        #[arg(long)]
        capability_id: Option<String>,
    },
    /// Remove a reference without cancelling or deleting its execution.
    Detach { id: String, execution_id: String },
    /// Dispatch a registered capability and attach the admitted execution.
    /// Body: {"execution_id","capability_id","input"?,"node_id"?}.
    Dispatch {
        id: String,
        #[arg(long)]
        json: String,
    },
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
        ProjectCmd::Tags(sub) => tags::plan(sub)?,
        ProjectCmd::Overview => RequestPlan::get("/api/project/overview"),
        ProjectCmd::Goals(sub) => plan_goals(sub)?,
        ProjectCmd::Initiatives(sub) => plan_initiatives(sub)?,
        ProjectCmd::Todos(sub) => plan_todos(sub)?,
    })
}

fn plan_goals(sub: &GoalsCmd) -> Result<RequestPlan> {
    Ok(match sub {
        GoalsCmd::Get { id } => {
            RequestPlan::get(format!("/api/project/goals/{}", crate::http::urlencode(id)))
        }
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

fn plan_initiatives(sub: &InitiativesCmd) -> Result<RequestPlan> {
    Ok(match sub {
        InitiativesCmd::Get { id } => RequestPlan::get(format!(
            "/api/project/initiatives/{}",
            crate::http::urlencode(id)
        )),
        InitiativesCmd::List { goal } => {
            RequestPlan::get("/api/project/initiatives").with_opt("goal_id", goal.clone())
        }
        InitiativesCmd::Create { json } => {
            RequestPlan::post("/api/project/initiatives").with_body(required_body(json)?)
        }
        InitiativesCmd::Patch { id, json } => {
            RequestPlan::patch(format!("/api/project/initiatives/{id}"))
                .with_body(required_body(json)?)
        }
        InitiativesCmd::Delete { id } => {
            RequestPlan::delete(format!("/api/project/initiatives/{id}"))
        }
    })
}

fn plan_todos(sub: &TodosCmd) -> Result<RequestPlan> {
    Ok(match sub {
        TodosCmd::Get { id } => {
            RequestPlan::get(format!("/api/project/todos/{}", crate::http::urlencode(id)))
        }
        TodosCmd::Reorder { json } => {
            RequestPlan::put("/api/project/todos/order").with_body(required_body(json)?)
        }
        TodosCmd::Prompt(_)
        | TodosCmd::Messages(_)
        | TodosCmd::Events(_)
        | TodosCmd::Result(_)
        | TodosCmd::Interrupt(_) => {
            anyhow::bail!("conversation commands resolve the TODO execution before planning")
        }
        TodosCmd::List { initiative } => {
            RequestPlan::get("/api/project/todos").with_opt("initiative_id", initiative.clone())
        }
        TodosCmd::Create { json } => {
            RequestPlan::post("/api/project/todos").with_body(required_body(json)?)
        }
        TodosCmd::Patch { id, json } => {
            RequestPlan::patch(format!("/api/project/todos/{id}")).with_body(required_body(json)?)
        }
        TodosCmd::Delete { id } => RequestPlan::delete(format!("/api/project/todos/{id}")),
        TodosCmd::Links { id } => RequestPlan::get(format!(
            "/api/project/todos/{}/executions",
            crate::http::urlencode(id)
        )),
        TodosCmd::Attach {
            id,
            execution_id,
            capability_id,
        } => RequestPlan::post(format!(
            "/api/project/todos/{}/executions",
            crate::http::urlencode(id)
        ))
        .with_body(serde_json::json!({"execution_id":execution_id,"capability_id":capability_id})),
        TodosCmd::Detach { id, execution_id } => RequestPlan::delete(format!(
            "/api/project/todos/{}/executions/{}",
            crate::http::urlencode(id),
            crate::http::urlencode(execution_id)
        )),
        TodosCmd::Dispatch { id, json } => RequestPlan::post(format!(
            "/api/project/todos/{}/dispatch",
            crate::http::urlencode(id)
        ))
        .with_body(required_body(json)?),
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
    if let ProjectCmd::Todos(command) = &sub {
        match command {
            TodosCmd::Prompt(args) => {
                return conversation::run(
                    ctx,
                    &args.target,
                    "prompt",
                    Some(required_body(&args.json)?),
                    vec![],
                )
                .await
            }
            TodosCmd::Messages(args) => {
                let mut query = vec![];
                if let Some(seq) = args.after_seq {
                    query.push(("after_seq", seq.to_string()));
                }
                if let Some(offset) = args.message_offset {
                    query.push(("message_offset", offset.to_string()));
                }
                return conversation::run(ctx, &args.target, "messages", None, query).await;
            }
            TodosCmd::Events(args) => {
                return conversation::run(
                    ctx,
                    &args.target,
                    "events",
                    None,
                    vec![("after", args.after.to_string())],
                )
                .await
            }
            TodosCmd::Result(target) => {
                return conversation::run(ctx, target, "result", None, vec![]).await
            }
            TodosCmd::Interrupt(target) => {
                return conversation::run(ctx, target, "interrupt", None, vec![]).await
            }
            _ => {}
        }
    }
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
    fn project_capability_tools_link_existing_sessions_and_dispatch_with_stable_ids() {
        let attach = plan_todos(&TodosCmd::Attach {
            id: "t1".into(),
            execution_id: "operator-existing".into(),
            capability_id: None,
        })
        .unwrap();
        assert_eq!(attach.path, "/api/project/todos/t1/executions");
        assert_eq!(attach.body.unwrap()["execution_id"], "operator-existing");
        assert_eq!(
            plan_todos(&TodosCmd::Links { id: "t1".into() })
                .unwrap()
                .method,
            reqwest::Method::GET
        );
        assert_eq!(
            plan_todos(&TodosCmd::Detach {
                id: "t1".into(),
                execution_id: "operator-existing".into()
            })
            .unwrap()
            .method,
            reqwest::Method::DELETE
        );
        let dispatch = plan_todos(&TodosCmd::Dispatch {
            id: "t1".into(),
            json:
                r#"{"execution_id":"agent-stable","capability_id":"cap","input":{"prompt":"task"}}"#
                    .into(),
        })
        .unwrap();
        assert_eq!(dispatch.path, "/api/project/todos/t1/dispatch");
        assert_eq!(dispatch.body.unwrap()["execution_id"], "agent-stable");
        assert!(plan_todos(&TodosCmd::Dispatch {
            id: "t1".into(),
            json: "invalid".into()
        })
        .is_err());
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
