//! Scoped device transport, issued by the host before a Codex step starts.
use super::{ExecDeps, StepCtx};
use anyhow::{ensure, Context, Result};
use opencoder_core::config::DagDeviceConfig;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

mod output;

const GUEST: &str = "/run/opencoder-device";

pub(super) struct Access {
    root: PathBuf,
    identity: Value,
    input: Value,
    config: DagDeviceConfig,
    ui: bool,
}

fn identity(ctx: &StepCtx, session_id: &str) -> Value {
    json!({"dag_id":ctx.run_id,"step_id":ctx.step.name,
        "instance_id":ctx.instance.map(|i|i.to_string()),"session_id":session_id})
}

fn controlled_input(ctx: &StepCtx) -> Result<Option<Value>> {
    let ui = ctx.spec.name == opencoder_dag::ui_cases::NAME;
    if !ui && ctx.spec.name != opencoder_dag::devices::NAME {
        return Ok(None);
    }
    let input: Value = serde_json::from_slice(&std::fs::read(
        ctx.workflow_root.join(&ctx.run_id).join("input.json"),
    )?)?;
    let expected = if ui {
        opencoder_dag::ui_cases::definition(&input)
    } else {
        opencoder_dag::devices::definition(&input)
    }
    .map_err(anyhow::Error::msg)?;
    ensure!(
        ctx.spec == expected,
        "Device authority requires the exact controlled workflow definition"
    );
    ensure!(
        (ctx.step.name == "allocate" && ctx.instance.is_none())
            || (ctx.step.name == "execute" && ctx.instance.is_some()),
        "Unknown device step identity"
    );
    Ok(Some(
        if ui {
            opencoder_dag::ui_cases::public_input(&input)
        } else {
            opencoder_dag::devices::public_input(&input)
        }
        .map_err(anyhow::Error::msg)?,
    ))
}

fn endpoint(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).context("Invalid device API endpoint")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Invalid device API endpoint"
    );
    Ok(url)
}

async fn request(
    config: &DagDeviceConfig,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = endpoint(&config.host_endpoint)?.join(path)?;
    let token = std::fs::read_to_string(&config.token_file)
        .context("Host device credential unavailable")?;
    ensure!(!token.trim().is_empty(), "Empty host device credential");
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut request = client.request(method, url).bearer_auth(token.trim());
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .context("Device API transport failed")?;
    ensure!(
        response.status().is_success(),
        "Device API rejected host request: HTTP {}",
        response.status().as_u16()
    );
    response.json().await.context("Invalid device API response")
}

fn assignment(ctx: &StepCtx, input: &Value) -> Result<Value> {
    let index = ctx.instance.context("Device execution needs an instance")?;
    let instance_id = index.to_string();
    let value: Value = serde_json::from_str(
        ctx.instance_input
            .as_ref()
            .and_then(Value::as_str)
            .context("Device assignment must be a JSON string")?,
    )?;
    ensure!(
        value["instance_id"] == instance_id,
        "Device assignment belongs to a different instance"
    );
    let ui = ctx.spec.name == opencoder_dag::ui_cases::NAME;
    let expected = if ui {
        json!(opencoder_dag::ui_cases::case_batch(input, index).map_err(anyhow::Error::msg)?)
    } else {
        json!(opencoder_dag::devices::case_batch(input, index).map_err(anyhow::Error::msg)?)
    };
    ensure!(
        value[if ui { "cases" } else { "case_ids" }] == expected,
        "Device case batch changed"
    );
    let machine = value["machine"]
        .as_str()
        .context("Device machine missing")?;
    ensure!(
        (2..=19).any(|i| machine == format!("win-{i:02}")),
        "Device outside allowed fleet"
    );
    let reservation = value["reservation_id"]
        .as_str()
        .context("Device reservation missing")?;
    ensure!(
        !reservation.is_empty()
            && reservation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
        "Invalid reservation ID"
    );
    ensure!(
        value["generation"].as_u64().is_some(),
        "Device generation missing"
    );
    Ok(value)
}

pub(super) async fn prepare(
    ctx: &StepCtx,
    deps: &ExecDeps,
    session_id: &str,
) -> Result<Option<Access>> {
    let Some(input) = controlled_input(ctx)? else {
        return Ok(None);
    };
    let config = deps
        .config
        .dag
        .device_manager
        .as_ref()
        .context("Device workflow requires host device_manager configuration")?;
    endpoint(&config.step_endpoint)?;
    let ui = ctx.spec.name == opencoder_dag::ui_cases::NAME;
    let harness_name = if ui {
        opencoder_dag::ui_cases::NAME
    } else {
        opencoder_dag::devices::NAME
    };
    let harness =
        opencoder_core::agent::scope::with_root_sync(deps.config.agent.agents_dir.clone(), || {
            opencoder_core::harness::agent_harness(harness_name)
        });
    ensure!(
        harness == opencoder_core::harness::Harness::Codex,
        "Device workflow requires its installed Codex agent"
    );
    let who = identity(ctx, session_id);
    let (scope, assignment) = if ctx.step.name == "allocate" {
        let mut body = json!({"dag_id":ctx.run_id,"step_id":ctx.step.name,"instance_id":null,
            "session_id":session_id,"role":"allocate","target_step":"execute","count":input["device_count"]});
        if ui {
            body["work_type"] = json!("ui");
            body["case_ids"] = json!(input["cases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["case_id"].as_str().unwrap())
                .collect::<Vec<_>>());
            body["case_specs"] = input["cases"].clone();
        }
        (
            request(
                config,
                reqwest::Method::POST,
                "/v1/step-sessions",
                Some(&body),
            )
            .await?,
            Value::Null,
        )
    } else {
        let assignment = assignment(ctx, &input)?;
        let id = assignment["reservation_id"].as_str().unwrap();
        let reservation = request(
            config,
            reqwest::Method::GET,
            &format!("/v1/reservations/{id}/status"),
            None,
        )
        .await?;
        ensure!(
            reservation["dag_id"] == ctx.run_id && reservation["target_step"] == ctx.step.name,
            "Device reservation belongs to another DAG or step"
        );
        let found = reservation["assignments"]
            .as_array()
            .context("Missing device assignments")?
            .iter()
            .find(|a| a["instance_id"] == assignment["instance_id"])
            .context("Device instance absent from reservation")?;
        ensure!(
            found["machine"] == assignment["machine"],
            "Device machine changed"
        );
        let body = json!({"instance_id":assignment["instance_id"],"generation":found["generation"],"session_id":session_id});
        let scope = request(
            config,
            reqwest::Method::POST,
            &format!("/v1/reservations/{id}/claims"),
            Some(&body),
        )
        .await?;
        let mut assignment = assignment;
        assignment["generation"] = scope["generation"].clone();
        ensure!(
            assignment["generation"].as_u64().is_some(),
            "Claim generation missing"
        );
        (scope, assignment)
    };
    let capability = scope["capability"]
        .as_str()
        .filter(|v| !v.is_empty())
        .context("Device scope capability missing")?;
    let root = ctx.workflow_root.join(".device-sessions").join(session_id);
    std::fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    }
    let transport = json!({"endpoint":config.step_endpoint,"capability":capability,"identity":who,
        "assignment":assignment,"input":input,
        "work_type":if ui { "ui" } else { "native" },
        "recovery_only":scope["recovery_only"].as_bool().unwrap_or(false),
        "works":scope.get("works").cloned().unwrap_or(json!([]))});
    let file = root.join("transport.json");
    opencoder_core::atomic_write(&file, &serde_json::to_vec(&transport)?)?;
    std::fs::write(root.join("client.py"), include_str!("client.py"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(Some(Access {
        root,
        identity: who,
        input,
        config: config.clone(),
        ui,
    }))
}

impl Access {
    pub(super) fn prompt(&self, prompt: String, container: bool) -> String {
        let root = if container {
            Path::new(GUEST)
        } else {
            self.root.as_path()
        };
        let transport = root.join("transport.json");
        let execution = if self.ui {
            "execute only the frozen UI cases through the scoped UI work transport".to_string()
        } else {
            format!(
                "execute cases with native-harness.py --device-context {}",
                transport.display()
            )
        };
        format!("{prompt}\n\nHost-authenticated step identity: {}\nDevice input: {}\nPrivate tool transport: {}. Never read its credential contents into context or logs. Use python3 {} reserve for allocate; {execution}. Only API results without credentials may be returned.",
            self.identity,self.input,transport.display(),root.join("client.py").display())
    }
    pub(super) fn bind(&self, bundle: &Path) -> Result<()> {
        super::private_files::bind_at(bundle, &self.root, GUEST)
    }
    async fn validate_completion(&self, output: Option<&Value>) -> Result<()> {
        let dag = self.identity["dag_id"]
            .as_str()
            .context("Host DAG identity missing")?;
        let id = output::reservation_id(dag);
        let reservation = request(
            &self.config,
            reqwest::Method::GET,
            &format!("/v1/reservations/{id}/status"),
            None,
        )
        .await?;
        if self.identity["step_id"] == "allocate" {
            if self.ui {
                output::validate_ui(
                    output.context("Allocator produced no structured output")?,
                    &reservation,
                    &self.input,
                    dag,
                )
            } else {
                output::validate(
                    output.context("Allocator produced no structured output")?,
                    &reservation,
                    &self.input,
                    dag,
                )
            }
        } else {
            if self.ui {
                output::completed_ui(&reservation, &self.input, &self.identity)
            } else {
                output::completed(&reservation, &self.input, &self.identity)
            }
        }
    }

    pub(super) async fn finish(&self, result: &mut super::StepResult) {
        if result.outcome == opencoder_dag::StepOutcome::Done {
            let checked = self.validate_completion(result.output_json.as_ref()).await;
            if let Err(error) = checked {
                result.outcome = opencoder_dag::StepOutcome::Error;
                result.error = Some(format!("Device completion verification failed: {error:#}"));
            }
        }
        let mut body = self.identity.clone();
        body["outcome"] = json!(match result.outcome {
            opencoder_dag::StepOutcome::Done => "done",
            opencoder_dag::StepOutcome::Cancelled => "cancelled",
            opencoder_dag::StepOutcome::Error => "error",
        });
        let path = format!(
            "/v1/step-sessions/{}/finish",
            self.identity["session_id"].as_str().unwrap()
        );
        if let Err(error) = request(&self.config, reqwest::Method::POST, &path, Some(&body)).await {
            result.outcome = opencoder_dag::StepOutcome::Error;
            result.error = Some(format!(
                "Host device session completion unconfirmed: {error:#}"
            ));
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transport_tests;
