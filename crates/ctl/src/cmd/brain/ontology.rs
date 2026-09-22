use super::required_body;
use crate::http::RequestPlan;
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum PlansCmd {
    List,
    Save {
        #[arg(long)]
        json: String,
    },
    Validate {
        #[arg(long)]
        json: String,
    },
    Get {
        id: String,
        version: u64,
    },
    Versions {
        id: String,
        #[arg(long)]
        before: Option<u64>,
    },
    Stable {
        id: String,
        version: u64,
    },
    Diff {
        id: String,
        from: u64,
        to: u64,
    },
}
#[derive(Subcommand, Debug)]
pub enum RunsCmd {
    List,
    Create {
        #[arg(long)]
        json: String,
    },
    Get {
        id: String,
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    /// Schema 4 layered canvas: run, plan, recomputed layers, operations.
    Layered {
        id: String,
    },
    /// Schema 4 layered round detail for one layer.
    LayeredRound {
        id: String,
        round: u32,
    },
    Events {
        id: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    Command {
        id: String,
        action: String,
    },
}
pub fn plans(command: &PlansCmd) -> Result<RequestPlan> {
    let base = "/api/brain/plan-defs";
    Ok(match command {
        PlansCmd::List => RequestPlan::get(base),
        PlansCmd::Save { json } => RequestPlan::post(base).with_body(required_body(json)?),
        PlansCmd::Validate { json } => {
            RequestPlan::post(format!("{base}/validate")).with_body(required_body(json)?)
        }
        PlansCmd::Get { id, version } => {
            RequestPlan::get(format!("{base}/{id}/versions/{version}"))
        }
        PlansCmd::Versions { id, before } => RequestPlan::get(format!(
            "{base}/{id}/versions{}",
            before.map(|v| format!("?before={v}")).unwrap_or_default()
        )),
        PlansCmd::Stable { id, version } => RequestPlan::post(format!("{base}/{id}/stable"))
            .with_body(serde_json::json!({"version":version})),
        PlansCmd::Diff { id, from, to } => {
            RequestPlan::get(format!("{base}/{id}/diff?from={from}&to={to}"))
        }
    })
}
pub fn runs(command: &RunsCmd) -> Result<RequestPlan> {
    let base = "/api/brain/runs";
    Ok(match command {
        RunsCmd::List => RequestPlan::get(base),
        RunsCmd::Create { json } => RequestPlan::post(base).with_body(scheduler_body(json)?),
        RunsCmd::Get { id, offset } => RequestPlan::get(format!("{base}/{id}?offset={offset}")),
        RunsCmd::Layered { id } => RequestPlan::get(format!("{base}/{id}/layered")),
        RunsCmd::LayeredRound { id, round } => {
            RequestPlan::get(format!("{base}/{id}/layered/rounds/{round}"))
        }
        RunsCmd::Events { id, after } => {
            RequestPlan::get(format!("{base}/{id}/events-page?after={after}"))
        }
        RunsCmd::Command { id, action } => RequestPlan::post(format!("{base}/{id}/commands"))
            .with_body(serde_json::json!({"action":action})),
    })
}

/// Only explicit layered requests reach the create endpoint.
fn scheduler_body(raw: &str) -> Result<serde_json::Value> {
    let body = required_body(raw)?;
    anyhow::ensure!(
        matches!(body["schema_version"].as_u64(), Some(4)),
        "brain runs create requires an explicit schema_version: 4"
    );
    Ok(body)
}

pub async fn activate(
    context: &std::path::Path,
    config: &std::path::Path,
    output: &std::path::Path,
) -> Result<i32> {
    let context: serde_json::Value = serde_json::from_slice(&std::fs::read(context)?)?;
    let config: opencoder_core::Config = serde_json::from_slice(&std::fs::read(config)?)?;
    let client = LocalClient(config.clone());
    let decision = if context["schema_version"] == 4 && context["nodes"] == serde_json::json!([]) {
        serde_json::to_value(
            closing_decision(
                &serde_json::from_value(context)?,
                &client,
                config.model_id(),
            )
            .await?,
        )?
    } else if context["schema_version"] == 4 {
        serde_json::to_value(
            opencoder_brain::layered::activate(
                &serde_json::from_value(context)?,
                &client,
                config.model_id(),
            )
            .await?,
        )?
    } else {
        anyhow::bail!("unsupported brain schema; expected 4");
    };
    opencoder_core::atomic_write_json(output, &decision)?;
    Ok(0)
}

struct LocalClient(opencoder_core::Config);

/// Closing instruction text. `nodes` is empty, so a dispatch_layer decision has
/// nothing left to schedule.
const CLOSING: &str =
    "Every layer of the plan has been dispatched and every node has a successful attempt. \
Return complete or fail; dispatch_layer is closed.";

/// Closing activation: the layer being decided is `total_layers + 1`, so only
/// completion or failure remain. Mirrors
/// `opencoder_worker::brain::container::closing_decision` so the in-container
/// and the node-local activation send the same request.
async fn closing_decision(
    context: &opencoder_core::brain::layered::LayeredContext,
    client: &dyn opencoder_llm::ChatStream,
    model: &str,
) -> Result<opencoder_core::brain::layered::LayeredDecision> {
    let instruction = serde_json::json!({
        "schema_version": opencoder_core::brain::layered::LAYERED_SCHEMA_VERSION,
        "closing": true,
        "instruction": CLOSING,
        "run_id": context.run_id,
        "generation": context.generation,
        "layer": context.layer,
        "total_layers": context.total_layers,
        "objective": context.request.plan.objective,
        "operations": context.operations,
        "summaries": context.summaries,
    });
    let mut stream = client.chat_stream(opencoder_llm::ChatRequest {
        purpose: opencoder_llm::RequestPurpose::Planning,
        model: model.into(),
        messages: vec![
            opencoder_llm::Message::system("layered-contract", opencoder_brain::layered::PROMPT),
            opencoder_llm::Message::user("layered-closing", instruction.to_string()),
        ],
        tools: vec![],
        tool_choice: None,
        temperature: Some(0.0),
        max_tokens: Some(16384),
        reasoning_effort: None,
        cache_salt: None,
    })?;
    while let Some(event) = stream.recv().await {
        match event {
            opencoder_llm::LlmEvent::Completed { text, .. } => {
                anyhow::ensure!(text.len() <= 256 * 1024, "layered decision exceeds 256 KiB");
                return serde_json::from_str(text.trim())
                    .map_err(|error| anyhow::anyhow!("layered decision is not JSON: {error}"));
            }
            opencoder_llm::LlmEvent::Error(error) => {
                anyhow::bail!("layered provider: {error}")
            }
            _ => {}
        }
    }
    anyhow::bail!("layered stream ended without completion")
}

#[cfg(test)]
mod tests;

impl opencoder_llm::ChatStream for LocalClient {
    fn chat_stream(
        &self,
        request: opencoder_llm::ChatRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        let endpoint = self.0.resolve_endpoint()?;
        opencoder_llm::ChatClient::from_config(&self.0, &endpoint)?.chat_stream(
            opencoder_brain::activation::configured_request(&self.0, request),
        )
    }
}
