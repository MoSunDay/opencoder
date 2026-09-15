//! The privileged Host starts only the installed release job template.
use super::Host;
use anyhow::{ensure, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::atomic::Ordering};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    action: String,
    release_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Controller {
    state_dir: PathBuf,
    unit_prefix: String,
}

fn job(controller: &Controller, request: &Request, current: &Value) -> Result<String> {
    ensure!(
        matches!(request.action.as_str(), "deploy" | "rollback"),
        "unknown release action"
    );
    ensure!(
        !request.release_id.is_empty()
            && request.release_id.len() <= 64
            && request
                .release_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid release identity"
    );
    ensure!(
        current["current"].as_str() == Some(&request.release_id),
        "signal came from a superseded server"
    );
    ensure!(
        controller.state_dir.is_absolute(),
        "controller state directory must be absolute"
    );
    ensure!(
        controller.unit_prefix.starts_with("opencoder-release-")
            && controller
                .unit_prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "invalid deployment unit prefix"
    );
    Ok(format!(
        "{}@{}--{}.service",
        controller.unit_prefix, request.action, request.release_id
    ))
}

impl Host {
    pub(super) async fn signal_deployment(&self, request: Request) -> Result<Value> {
        ensure!(!self.retiring.load(Ordering::SeqCst), "Host is retiring");
        let current = self
            .store
            .definition("host", "current")
            .await?
            .unwrap_or_default();
        ensure!(current["instance"] == self.instance, "Host is not current");
        let controller: Controller =
            serde_json::from_slice(&tokio::fs::read(self.data_dir.join("deployment.json")).await?)?;
        let journal: Value = serde_json::from_slice(
            &tokio::fs::read(controller.state_dir.join("release-state.json")).await?,
        )?;
        let unit = job(&controller, &request, &journal)?;
        // systemd owns the new process/cgroup. This Host may retire before the
        // deployment finishes; ordinary parent/child supervision would kill it.
        let status = tokio::process::Command::new("systemctl")
            .args(["start", "--no-block", &unit])
            .status()
            .await?;
        ensure!(
            status.success(),
            "systemd did not accept the deployment job"
        );
        Ok(json!({"accepted":true,"action":request.action,"unit":unit}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_server_can_start_the_two_fixed_jobs() {
        let controller = Controller {
            state_dir: "/state".into(),
            unit_prefix: "opencoder-release-fixture".into(),
        };
        let current = json!({"current":"r2"});
        for action in ["deploy", "rollback"] {
            assert_eq!(
                job(
                    &controller,
                    &Request {
                        action: action.into(),
                        release_id: "r2".into()
                    },
                    &current
                )
                .unwrap(),
                format!("opencoder-release-fixture@{action}--r2.service")
            );
        }
        for (action, release_id) in [("deploy", "r1"), ("stop", "r2"), ("../runtime", "r2")] {
            assert!(job(
                &controller,
                &Request {
                    action: action.into(),
                    release_id: release_id.into()
                },
                &current
            )
            .is_err());
        }
        let invalid = Controller {
            unit_prefix: "opencoder-release-x --all".into(),
            ..controller
        };
        assert!(job(
            &invalid,
            &Request {
                action: "deploy".into(),
                release_id: "r2".into()
            },
            &current
        )
        .is_err());
    }
}
