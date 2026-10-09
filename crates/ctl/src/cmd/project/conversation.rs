//! Resolve a TODO's current execution once, then use native execution APIs.
use crate::{
    cmd::{exec_plan, exec_stream},
    ctx::Ctx,
    http::{self, RequestPlan},
    out,
};
use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};

#[derive(Args, Debug)]
pub struct Target {
    pub id: String,
    /// Use this linked execution instead of the current capability's latest one.
    #[arg(long)]
    pub execution: Option<String>,
}

#[derive(Args, Debug)]
pub struct Prompt {
    #[command(flatten)]
    pub target: Target,
    /// Input object: {"prompt":"...","input_id":"stable-id","delivery":"steer"|"queue"}.
    #[arg(long)]
    pub json: String,
}

#[derive(Args, Debug)]
pub struct Messages {
    #[command(flatten)]
    pub target: Target,
    #[arg(long)]
    pub after_seq: Option<i64>,
    #[arg(long)]
    pub message_offset: Option<u64>,
}

#[derive(Args, Debug)]
pub struct Events {
    #[command(flatten)]
    pub target: Target,
    #[arg(long, default_value_t = 0)]
    pub after: i64,
}

pub fn select_execution<'a>(
    body: &'a Value,
    requested: Option<&str>,
) -> Result<(&'a str, &'a str)> {
    let id = requested
        .or_else(|| body["current_execution_id"].as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("TODO has no current execution; use project todos dispatch first")
        })?;
    let assignment = body["assignments"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["execution_id"] == id))
        .ok_or_else(|| anyhow::anyhow!("execution is not linked to this TODO"))?;
    Ok((
        assignment["execution_id"].as_str().unwrap(),
        assignment["kind"].as_str().unwrap_or(""),
    ))
}

pub async fn run(
    ctx: &Ctx,
    target: &Target,
    operation: &str,
    input: Option<Value>,
    query: Vec<(&str, String)>,
) -> Result<i32> {
    let request = RequestPlan::get(format!(
        "/api/project/todos/{}/executions",
        http::urlencode(&target.id)
    ));
    let response = match http::send(ctx, &request).await {
        Ok(response) => response,
        Err(error) => {
            out::fail_transport(&error.to_string());
            return Ok(1);
        }
    };
    if !response.is_success() {
        out::fail(response.status, &response.error_message());
        return Ok(http::exit_code(response.status));
    }
    let body = response.json.unwrap_or(Value::Null);
    let (id, kind) = match select_execution(&body, target.execution.as_deref()) {
        Ok(selected) => selected,
        Err(error) => {
            out::fail(409, &error.to_string());
            return Ok(4);
        }
    };
    if matches!(operation, "prompt" | "interrupt" | "messages")
        && !matches!(kind, "agent" | "operator")
    {
        out::fail(409, "this capability does not support a conversation");
        return Ok(4);
    }
    let id = http::urlencode(id);
    let mut request = match operation {
        "prompt" => {
            let mut input = input.unwrap_or(json!({}));
            anyhow::ensure!(
                input["prompt"]
                    .as_str()
                    .is_some_and(|text| !text.trim().is_empty()),
                "prompt is required"
            );
            if input.get("input_id").is_none() {
                input["input_id"] = json!(format!("input-{}", ulid::Ulid::new()));
            }
            let action = input
                .get("delivery")
                .and_then(Value::as_str)
                .unwrap_or("steer")
                .to_owned();
            anyhow::ensure!(
                matches!(action.as_str(), "steer" | "queue"),
                "delivery must be steer or queue"
            );
            input.as_object_mut().unwrap().remove("delivery");
            RequestPlan::post(format!("/api/executions/{id}/commands"))
                .with_body(json!({"action":action,"input":input}))
        }
        "interrupt" => RequestPlan::post(format!("/api/executions/{id}/commands"))
            .with_body(json!({"action":"interrupt","input":{}})),
        resource => RequestPlan::get(format!("/api/executions/{id}/{resource}")),
    };
    for (key, value) in query {
        request = request.with(key, value);
    }
    if operation == "events" {
        exec_stream(ctx, request).await
    } else {
        exec_plan(ctx, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_only_linked_executions_and_keeps_explicit_history() {
        let body = json!({"current_execution_id":"a","assignments":[{"execution_id":"a","kind":"agent"},{"execution_id":"d","kind":"dag"}]});
        assert_eq!(select_execution(&body, None).unwrap(), ("a", "agent"));
        assert_eq!(select_execution(&body, Some("d")).unwrap(), ("d", "dag"));
        assert!(select_execution(&body, Some("foreign")).is_err());
        assert!(select_execution(&json!({"assignments":[]}), None).is_err());
    }
}
