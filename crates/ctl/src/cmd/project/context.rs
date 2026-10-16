//! Recover a project TODO's context without changing it or its executions.
use super::{comments, summary};
use crate::{
    ctx::Ctx,
    http::{self, RequestPlan},
    out,
};
use anyhow::{Context, Result};
use clap::Args;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Args, Debug)]
pub struct Options {
    pub id: String,
    /// Read only this linked execution; otherwise include the entire history.
    #[arg(long)]
    pub execution: Option<String>,
}

pub async fn collect(ctx: &Ctx, args: &Options) -> Result<Value> {
    let base = format!("/api/project/todos/{}", http::urlencode(&args.id));
    let mut record = summary::read(ctx, RequestPlan::get(&base)).await?;
    let links = summary::read(ctx, RequestPlan::get(format!("{base}/executions"))).await?;
    let assignments = links["assignments"]
        .as_array()
        .context("missing execution assignments")?;
    if let Some(id) = &args.execution {
        if !assignments.iter().any(|row| row["execution_id"] == *id) {
            return Err(summary::ReadError {
                status: 409,
                message: "execution is not linked to this TODO".into(),
            }
            .into());
        }
    }
    let mut issues = vec![];
    record["comments"] = match comments::all(ctx, &args.id).await {
        Ok(value) => value,
        Err(error) => {
            issues.push(json!({"source":format!("{base}/comments"),"error":error.to_string()}));
            Value::Null
        }
    };
    let mut executions = BTreeMap::new();
    for assignment in assignments {
        let id = assignment["execution_id"]
            .as_str()
            .context("assignment has no execution id")?;
        if args.execution.as_deref().is_some_and(|chosen| chosen != id)
            || executions.contains_key(id)
        {
            continue;
        }
        let evidence = summary::paging::execution(
            ctx,
            id,
            assignment["kind"].as_str().unwrap_or(""),
            &mut issues,
        )
        .await;
        executions.insert(id.to_owned(), evidence);
    }
    Ok(json!({
        "scope":"todo", "record":record, "assignments":assignments,
        "current_execution_id":links["current_execution_id"],
        "selected_execution_id":args.execution, "executions":executions,
        "complete":issues.is_empty(), "issues":issues,
        "collected_at":opencoder_core::message::now_ms(),
        "guidance":"执行详情、消息、过程和完整字段来自所属节点；运行中的执行仍可能产生新进展。complete 仅表示本次所选证据读取完整，不代表执行成功或 TODO 已验收。"
    }))
}

pub async fn run(ctx: &Ctx, args: &Options) -> Result<i32> {
    match collect(ctx, args).await {
        Ok(value) => {
            let code = if value["complete"] == true { 0 } else { 4 };
            out::json(&value);
            Ok(code)
        }
        Err(error) => Ok(summary::fail(&error)),
    }
}
