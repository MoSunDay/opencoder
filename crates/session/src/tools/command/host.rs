//! Non-interactive host shell selection and short commands used by TUI hooks.
use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

pub async fn program() -> Result<PathBuf> {
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("bash"))
    }
    #[cfg(windows)]
    {
        use std::sync::OnceLock;
        static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
        if let Some(path) = PROGRAM.get() {
            return Ok(path.clone());
        }
        let mut candidates: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|path| path.join("pwsh.exe"))
                .collect();
        if let Some(directory) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(directory).join("PowerShell/7/pwsh.exe"));
        }
        let path = candidates.into_iter().find(|path| path.is_file()).context(
            "PowerShell 7.4 or newer is required: install Microsoft.PowerShell and put pwsh.exe on PATH",
        )?;
        let output = run_program(
            &path,
            "$PSVersionTable.PSVersion.ToString()",
            None,
            Duration::from_secs(3),
        )
        .await
        .context("PowerShell version check failed")?;
        anyhow::ensure!(
            output.status.success()
                && supported_powershell_version(&String::from_utf8_lossy(&output.stdout)),
            "PowerShell 7.4 or newer in the 7.x series is required; install a stable PowerShell release"
        );
        let _ = PROGRAM.set(path.clone());
        Ok(path)
    }
}

#[cfg(any(windows, test))]
fn supported_powershell_version(version: &str) -> bool {
    let version = version.trim();
    let mut parts = version.split('.');
    parts.next() == Some("7")
        && parts
            .next()
            .and_then(|minor| minor.parse::<u32>().ok())
            .is_some_and(|minor| minor >= 4)
        && !version.contains('-')
}

pub fn arguments(command: &mut Command, script: &str) {
    #[cfg(not(windows))]
    {
        command.args(["-lc", script]);
    }
    #[cfg(windows)]
    {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &STANDARD.encode(bytes),
        ]);
    }
}

pub async fn run(
    script: &str,
    workdir: Option<&Path>,
    timeout: Duration,
) -> Result<std::process::Output> {
    let program = program().await?;
    let script = if cfg!(windows) {
        super::powershell::script(script)
    } else {
        script.to_owned()
    };
    run_program(&program, &script, workdir, timeout).await
}

async fn run_program(
    program: &Path,
    script: &str,
    workdir: Option<&Path>,
    timeout: Duration,
) -> Result<std::process::Output> {
    let (mut command, lease) = match crate::process::command(program)? {
        Some((command, lease)) => (command, Some(lease)),
        None => (Command::new(program), None),
    };
    arguments(&mut command, script);
    if let Some(workdir) = workdir {
        command.current_dir(workdir);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::configure_owned_command(&mut command, lease.is_some());
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn()?;
    #[cfg(unix)]
    let _group = DirectGroup(if lease.is_none() { child.id() } else { None });
    let mut owner = lease.map(|lease| lease.spawned(child.id())).transpose()?;
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;
    if let Some(owner) = &mut owner {
        owner.terminate();
    }
    result
        .context("host command timed out")?
        .context("host command failed")
}

#[cfg(unix)]
struct DirectGroup(Option<u32>);
#[cfg(unix)]
impl Drop for DirectGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

#[cfg(test)]
mod version_tests {
    use super::supported_powershell_version;

    #[test]
    fn native_argument_policy_requires_a_stable_supported_powershell() {
        for version in ["7.4.0", "7.4.6\n", "7.6.6"] {
            assert!(supported_powershell_version(version), "{version}");
        }
        for version in [
            "5.1.0",
            "7.0.0",
            "7.2.0",
            "7.3.0",
            "7.4.0-preview.1",
            "8.0.0",
            "invalid",
        ] {
            assert!(!supported_powershell_version(version), "{version}");
        }
    }
}
