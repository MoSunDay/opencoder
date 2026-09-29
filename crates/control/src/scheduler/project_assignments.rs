use crate::{api::executions, AppState};
use anyhow::Result;
use futures::{stream, StreamExt};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus, NodeOperation};
use opencoder_store::project::ProjectAssignment;
use serde_json::Value;
use std::sync::Arc;

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() && value.trim() != "null" => {
            Some(value.trim().to_owned())
        }
        Value::Object(value) if !value.is_empty() && !value.contains_key("omitted") => {
            Some(serde_json::to_string_pretty(value).ok()?)
        }
        _ => None,
    }
}

fn bounded(value: String) -> String {
    value.chars().take(64 * 1024).collect()
}

fn direct_conclusion(kind: ExecutionKind, detail: &Value) -> Option<String> {
    let result = &detail["result"];
    match kind {
        ExecutionKind::Agent | ExecutionKind::Operator => text(&result["output_text"]),
        ExecutionKind::Team => {
            text(&detail["topic"]["final_summary"]).or_else(|| text(&result["final_summary"]))
        }
        ExecutionKind::Todos => result["todos"].as_object().and_then(|todos| {
            let rows: Vec<String> = todos
                .iter()
                .filter_map(|(name, todo)| {
                    text(&todo["candidate"]["result"])
                        .or_else(|| text(&todo["candidate"]["summary"]))
                        .map(|value| format!("### {name}\n{value}"))
                })
                .collect();
            (!rows.is_empty()).then(|| rows.join("\n\n"))
        }),
        _ => None,
    }
}

async fn conclusion(
    state: &AppState,
    record: &ProjectAssignment,
    kind: ExecutionKind,
) -> Result<Option<String>> {
    let reply = executions::inspect_id(state, &record.execution_id).await;
    if reply.status >= 300 {
        anyhow::bail!("inspect {}: {}", record.execution_id, reply.status);
    }
    let detail = reply.body;
    let result = &detail["result"];
    let output = match kind {
        ExecutionKind::Agent
        | ExecutionKind::Operator
        | ExecutionKind::Team
        | ExecutionKind::Todos => direct_conclusion(kind, &detail),
        ExecutionKind::Brain => {
            let reply = executions::for_id(state, &record.execution_id, |execution| {
                NodeOperation::Brain {
                    execution,
                    action: "layered_summary".into(),
                    input: Value::Null,
                }
            })
            .await;
            if reply.status < 300 {
                text(&reply.body["summary"])
            } else {
                text(&result["scheduler_output"])
            }
        }
        ExecutionKind::Dag => {
            let mut rows = Vec::new();
            for step in detail["dag_steps"]["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .take(16)
            {
                let Some(name) = step["name"].as_str() else {
                    continue;
                };
                let reply = executions::for_id(state, &record.execution_id, |execution| {
                    NodeOperation::DagSteps {
                        execution,
                        step: Some(name.to_owned()),
                    }
                })
                .await;
                if reply.status < 300 {
                    if let Some(value) = text(&reply.body["output"]) {
                        rows.push(format!("### {name}\n{value}"));
                    }
                }
            }
            (!rows.is_empty()).then(|| rows.join("\n\n"))
        }
        _ => None,
    };
    Ok(output.map(bounded))
}

async fn sync(state: &AppState, record: &ProjectAssignment) -> Result<()> {
    let Some(index) = state.fleet.index(&record.execution_id).await? else {
        return Ok(());
    };
    let ready = index.status == ExecutionStatus::Done
        || (matches!(index.kind, ExecutionKind::Agent | ExecutionKind::Operator)
            && index.status == ExecutionStatus::Idle);
    let state_name = if ready {
        let output = conclusion(state, record, index.kind).await?;
        if output.is_none()
            && (index.status == ExecutionStatus::Idle
                || opencoder_core::message::now_ms() - record.created_at < 120_000)
        {
            return Ok(());
        }
        let state_name = if output.is_some() {
            "complete"
        } else {
            "empty"
        };
        state
            .projects
            .finish_todo_assignment(
                &record.todo_id,
                &record.execution_id,
                state_name,
                output.as_deref(),
            )
            .await?;
        return Ok(());
    } else if index.status == ExecutionStatus::Error {
        "error"
    } else if index.status == ExecutionStatus::Cancelled {
        "cancelled"
    } else {
        return Ok(());
    };
    state
        .projects
        .finish_todo_assignment(&record.todo_id, &record.execution_id, state_name, None)
        .await?;
    Ok(())
}

pub async fn scan(state: &Arc<AppState>, after: &str) -> Result<String> {
    let records = state.projects.pending_todo_assignments(after, 25).await?;
    let Some(last) = records.last() else {
        return Ok(String::new());
    };
    let next = format!("{}:{}", last.todo_id, last.execution_id);
    stream::iter(records.iter())
        .for_each_concurrent(10, |record| async move {
            match tokio::time::timeout(std::time::Duration::from_secs(20), sync(state, record)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(execution_id = %record.execution_id, %error, "project assignment sync failed"),
                Err(_) => tracing::warn!(execution_id = %record.execution_id, "project assignment sync timed out"),
            }
        })
        .await;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_conclusions_use_their_authoritative_result_fields() {
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Agent,
                &json!({"result":{"output_text":" agent answer "}})
            ),
            Some("agent answer".into())
        );
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Operator,
                &json!({"result":{"output_text":"operator answer"}})
            ),
            Some("operator answer".into())
        );
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Team,
                &json!({"topic":{"final_summary":"team answer"},"result":{"final_summary":"older"}})
            ),
            Some("team answer".into())
        );
        assert_eq!(
            direct_conclusion(
                ExecutionKind::Todos,
                &json!({"result":{"todos":{"work":{"candidate":{"result":"done","summary":"summary"}}}}})
            ),
            Some("### work\ndone".into())
        );
    }

    #[test]
    fn missing_native_conclusions_do_not_claim_success() {
        for kind in [
            ExecutionKind::Agent,
            ExecutionKind::Team,
            ExecutionKind::Todos,
        ] {
            assert_eq!(direct_conclusion(kind, &json!({"result":{}})), None);
        }
    }
}
