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
    Round {
        id: String,
        round: u32,
    },
    Context {
        id: String,
    },
    Actions {
        id: String,
    },
    Instance {
        id: String,
        instance: String,
    },
    Events {
        id: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    Input {
        id: String,
        #[arg(long)]
        json: String,
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
        RunsCmd::Create { json } => RequestPlan::post(base).with_body(required_body(json)?),
        RunsCmd::Get { id, offset } => RequestPlan::get(format!("{base}/{id}?offset={offset}")),
        RunsCmd::Round { id, round } => RequestPlan::get(format!("{base}/{id}/rounds/{round}")),
        RunsCmd::Context { id } => RequestPlan::get(format!("{base}/{id}/context")),
        RunsCmd::Actions { id } => RequestPlan::get(format!("{base}/{id}/actions")),
        RunsCmd::Instance { id, instance } => {
            RequestPlan::get(format!("{base}/{id}/instances/{instance}"))
        }
        RunsCmd::Events { id, after } => {
            RequestPlan::get(format!("{base}/{id}/events-page?after={after}"))
        }
        RunsCmd::Input { id, json } => {
            RequestPlan::post(format!("{base}/{id}/inputs")).with_body(required_body(json)?)
        }
        RunsCmd::Command { id, action } => RequestPlan::post(format!("{base}/{id}/commands"))
            .with_body(serde_json::json!({"action":action})),
    })
}

pub async fn activate(
    context: &std::path::Path,
    config: &std::path::Path,
    output: &std::path::Path,
) -> Result<i32> {
    let context: serde_json::Value = serde_json::from_slice(&std::fs::read(context)?)?;
    let config: opencoder_core::Config = serde_json::from_slice(&std::fs::read(config)?)?;
    let client = LocalClient(config.clone());
    let decision = if context["schema_version"] == 3 {
        serde_json::to_value(
            opencoder_brain::scheduler::activate(
                &serde_json::from_value(context)?,
                &client,
                config.model_id(),
            )
            .await?,
        )?
    } else {
        serde_json::to_value(
            opencoder_brain::activation::activate(
                &serde_json::from_value(context)?,
                &client,
                config.model_id(),
            )
            .await?,
        )?
    };
    opencoder_core::atomic_write_json(output, &decision)?;
    Ok(0)
}

struct LocalClient(opencoder_core::Config);
impl opencoder_llm::ChatStream for LocalClient {
    fn chat_stream(
        &self,
        request: opencoder_llm::ChatRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        let endpoint = self.0.resolve_endpoint()?;
        opencoder_llm::ChatClient::from_config(&self.0, &endpoint)?.chat_stream(request)
    }
}
