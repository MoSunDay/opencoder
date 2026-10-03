//! PowerShell parsing never evaluates the script being inspected.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

pub fn script(command: &str) -> String {
    format!("$ErrorActionPreference = 'Stop'\n[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)\n[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)\n$OutputEncoding = [Console]::OutputEncoding\ntry {{ & {{\n{command}\n}}; if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }} }} catch {{ [Console]::Error.WriteLine($_.ToString()); exit 1 }}")
}

#[derive(Deserialize)]
struct Inspection {
    #[serde(default)]
    commands: Vec<Invocation>,
    error: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct Invocation {
    pub(super) name: String,
    pub(super) args: Vec<String>,
    #[serde(default)]
    pub(super) start: usize,
    #[serde(default)]
    pub(super) end: usize,
    #[serde(default)]
    pub(super) name_start: usize,
    #[serde(default)]
    pub(super) name_end: usize,
}

pub async fn read_only(command: &str, workdir: &Path) -> Result<()> {
    inspect(command, workdir).await.map(|_| ())
}

/// The checked command is prepared for execution; callers must execute this
/// result rather than the original command after a successful inspection.
pub async fn prepare_read_only(command: &str, workdir: &Path) -> Result<String> {
    let inspection = inspect(command, workdir).await?;
    super::readonly::prepare(command, &inspection.commands)
}

async fn inspect(command: &str, workdir: &Path) -> Result<Inspection> {
    #[cfg(windows)]
    let program = super::host::program().await?;
    #[cfg(not(windows))]
    let program = std::path::PathBuf::from("pwsh");
    let (mut child, lease) = match crate::process::command(&program)? {
        Some((child, lease)) => (child, Some(lease)),
        None => (tokio::process::Command::new(&program), None),
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    let bytes: Vec<u8> = script(include_str!("inspect.ps1"))
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    child.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
        &STANDARD.encode(bytes),
    ]);
    child
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::configure_owned_command(&mut child, lease.is_some());
    let mut child = child.spawn()?;
    let mut owner = lease.map(|lease| lease.spawned(child.id())).transpose()?;
    let mut input = child
        .stdin
        .take()
        .context("PowerShell inspection stdin missing")?;
    input
        .write_all(serde_json::to_string(command)?.as_bytes())
        .await?;
    input.shutdown().await?;
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output()).await;
    if let Some(owner) = &mut owner {
        owner.terminate();
    }
    let output = output.context("PowerShell read-only inspection timed out")??;
    anyhow::ensure!(
        output.status.success(),
        "PowerShell read-only inspection failed"
    );
    let inspection: Inspection =
        serde_json::from_slice(&output.stdout).context("invalid PowerShell inspection response")?;
    if let Some(error) = &inspection.error {
        anyhow::bail!("{error}");
    }
    for invocation in &inspection.commands {
        super::readonly::validate(invocation)?;
    }
    Ok(inspection)
}

#[cfg(test)]
mod tests {
    use super::super::readonly::validate as validate_invocation;
    use super::*;
    #[test]
    fn read_only_commands_reject_native_write_and_execution_options() {
        let call = |name: &str, args: &[&str]| Invocation {
            name: name.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            start: 0,
            end: 0,
            name_start: 0,
            name_end: 0,
        };
        assert!(validate_invocation(&call("Get-Content", &["file.txt"])).is_ok());
        assert!(validate_invocation(&call("git", &["status"])).is_ok());
        for args in [
            vec!["checkout", "main"],
            vec!["diff", "--output=changed"],
            vec!["-c", "alias.x=!touch f", "x"],
            vec!["diff", "--ext-diff"],
        ] {
            assert!(validate_invocation(&call("git", &args)).is_err());
        }
        assert!(validate_invocation(&call("rg", &["--pre=evil", "x"])).is_err());
        assert!(validate_invocation(&call("Set-Content", &["file", "text"])).is_err());
    }
}
