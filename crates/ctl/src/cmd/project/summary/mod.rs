//! Read-only evidence collection. The caller authors and PATCHes the overview.
mod paging;

use crate::{
    ctx::Ctx,
    http::{self, RequestPlan},
    out,
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn versions_plan(collection: &str, id: &str) -> RequestPlan {
    RequestPlan::get(format!(
        "/api/project/{collection}/{}/summary-versions",
        http::urlencode(id)
    ))
}

pub(super) fn restore_plan(collection: &str, id: &str, version: i64) -> Result<RequestPlan> {
    anyhow::ensure!(version > 0, "summary version must be positive");
    Ok(RequestPlan::post(format!(
        "/api/project/{collection}/{}/summary-versions/{version}/restore",
        http::urlencode(id)
    )))
}

pub(super) async fn read(ctx: &Ctx, plan: RequestPlan) -> Result<Value> {
    let reply = tokio::time::timeout(std::time::Duration::from_secs(30), http::send(ctx, &plan))
        .await
        .context("evidence read timed out")??;
    if !reply.is_success() {
        return Err(ReadError {
            status: reply.status,
            message: reply.error_message(),
        }
        .into());
    }
    reply.json.context("server response is not JSON")
}

#[derive(Debug)]
struct ReadError {
    status: u16,
    message: String,
}
impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)
    }
}
impl std::error::Error for ReadError {}

fn selected(overview: &Value, kind: &str, id: &str) -> Result<Value> {
    let goals = overview["goals"].as_array().context("missing goals")?;
    let record = if kind == "project" {
        goals.iter().find(|row| row["id"] == id)
    } else {
        goals
            .iter()
            .flat_map(|goal| goal["initiatives"].as_array().into_iter().flatten())
            .chain(
                overview["standalone_initiatives"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
            .find(|row| row["id"] == id)
    };
    record.cloned().ok_or_else(|| {
        ReadError {
            status: 404,
            message: format!("{kind} not found: {id}"),
        }
        .into()
    })
}

/// Load every child, every assignment, and the native evidence behind each one.
pub async fn collect(ctx: &Ctx, kind: &str, id: &str) -> Result<Value> {
    anyhow::ensure!(
        matches!(kind, "project" | "initiative"),
        "invalid summary scope"
    );
    let overview = read(ctx, RequestPlan::get("/api/project/overview")).await?;
    let mut record = selected(&overview, kind, id)?;
    let mut issues = vec![];
    let mut executions = BTreeMap::<String, Value>::new();
    if kind == "initiative" {
        for todo in record["todos"].as_array_mut().context("missing TODOs")? {
            let todo_id = todo["id"].as_str().context("TODO has no id")?;
            let path = format!("/api/project/todos/{}/executions", http::urlencode(todo_id));
            match read(ctx, RequestPlan::get(&path)).await {
                Ok(links) => {
                    let assignments = links["assignments"]
                        .as_array()
                        .context("missing assignments")?;
                    for assignment in assignments {
                        let execution_id = assignment["execution_id"]
                            .as_str()
                            .context("assignment has no execution id")?;
                        if !executions.contains_key(execution_id) {
                            let evidence = paging::execution(
                                ctx,
                                execution_id,
                                assignment["kind"].as_str().unwrap_or(""),
                                &mut issues,
                            )
                            .await;
                            executions.insert(execution_id.into(), evidence);
                        }
                    }
                    todo["assignments"] = json!(assignments);
                }
                Err(error) => issues.push(json!({"source":path,"error":error.to_string()})),
            }
        }
    } else {
        // Project synthesis uses all saved initiative summaries and current TODO
        // counts. Keep TODO facts to expose missing or outdated summaries.
        for initiative in record["initiatives"]
            .as_array()
            .context("missing initiatives")?
        {
            let updated = initiative["summary_updated_at"].as_i64();
            if initiative["current_summary"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            {
                issues.push(json!({"initiative_id":initiative["id"],"error":"专项尚未填写当前概述，请先读取该专项 summary-context 并总结"}));
            } else if initiative["todos"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|todo| todo["updated_at"].as_i64() > updated)
            {
                issues.push(json!({"initiative_id":initiative["id"],"error":"TODO 在专项概述之后发生变化，请先更新专项概述"}));
            }
        }
    }
    Ok(json!({
        "scope":kind,"record":record,"executions":executions,"complete":issues.is_empty(),"issues":issues,
        "collected_at":opencoder_core::message::now_ms(),
        "guidance":"依据全部子项与执行证据，按关键进展、核心工作、剩余卡点整理当前概述；区分已完成、进行中与待验收。说明证据缺失，不能把执行成功当作 TODO 验收通过。项目概述先更新全部专项；执行可能仍在推进，保存前复核。使用 patch --json 写入 current_summary，服务器记录概述更新时间。"
    }))
}

pub async fn run(ctx: &Ctx, kind: &str, id: &str) -> Result<i32> {
    match collect(ctx, kind, id).await {
        Ok(value) => {
            out::json(&value);
            Ok(0)
        }
        Err(error) => {
            if let Some(error) = error.downcast_ref::<ReadError>() {
                out::fail(error.status, &error.message);
                Ok(http::exit_code(error.status))
            } else {
                out::fail_transport(&error.to_string());
                Ok(1)
            }
        }
    }
}
