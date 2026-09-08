//! Domain command modules. Each module owns a clap `Subcommand` enum plus a
//! pure `plan()` mapping (subcommand → `RequestPlan`, no side effects) and a
//! thin `run(ctx, sub)` executor. This module hosts the shared executors:
//! buffered (`exec_plan`) and streaming (`exec_stream` + its helpers).

pub mod agents;
pub mod brain;
pub mod dag;
pub mod executions;
pub mod nodes;
pub mod project;
pub mod raw;
pub mod sessions;
pub mod system;
pub mod teams;
pub mod todo;

use anyhow::Result;

use crate::{ctx::Ctx, http::RequestPlan, out};

/// Execute a plan and apply the output/exit-code contract:
/// stdout JSON on success; `{"status":..,"error":..}` on stderr with exit
/// code 1 (transport) / 2 (auth) / 4 (server rejection) otherwise.
pub async fn exec_plan(ctx: &Ctx, plan: RequestPlan) -> Result<i32> {
    if ctx.verbose {
        out::note(format!("-> {} {}", plan.method, plan.url(&ctx.server)));
    }
    let outcome = match crate::http::send(ctx, &plan).await {
        Ok(outcome) => outcome,
        Err(error) => {
            out::fail_transport(&format!("{error:#}"));
            return Ok(1);
        }
    };
    if outcome.is_success() {
        match outcome.json {
            Some(value) => out::json(&value),
            None if outcome.text.trim().is_empty() => out::json(&serde_json::json!({"ok": true})),
            None => out::json(&serde_json::json!({
                "ok": true,
                "body": outcome.text.trim(),
            })),
        }
        return Ok(0);
    }
    out::fail(outcome.status, &outcome.error_message());
    Ok(crate::http::exit_code(outcome.status))
}

/// Report a non-2xx streaming response via the structured stderr contract
/// and map it onto the exit-code contract (2 auth / 4 rejection).
pub(crate) async fn stream_failure(response: reqwest::Response) -> i32 {
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let error = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| text.trim().to_owned());
    out::fail(status, &error);
    crate::http::exit_code(status)
}

/// Send a streaming request, printing the verbose request line and turning
/// transport errors into the structured stderr contract (exit 1).
pub(crate) async fn open_stream(ctx: &Ctx, request: &RequestPlan) -> Result<reqwest::Response> {
    if ctx.verbose {
        out::note(format!(
            "-> {} {}",
            request.method,
            request.url(&ctx.server)
        ));
    }
    crate::http::send_streaming(ctx, request).await
}

/// Stream a plan as SSE frames: transport errors exit 1, non-2xx responses
/// keep the stderr JSON + exit-code contract (2 auth / 4 rejection), success
/// prints one JSON line per frame and exits 0.
pub(crate) async fn exec_stream(ctx: &Ctx, request: RequestPlan) -> Result<i32> {
    let response = match open_stream(ctx, &request).await {
        Ok(response) => response,
        Err(error) => {
            out::fail_transport(&format!("{error:#}"));
            return Ok(1);
        }
    };
    if !response.status().is_success() {
        return Ok(stream_failure(response).await);
    }
    crate::sse::print_stream(response).await?;
    Ok(0)
}
