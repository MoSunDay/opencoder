//! Codex launch contract for the OCI runner. Credentials remain in the node's
//! original Codex home; only paths and private settings cross the bundle seam.
mod bundle;
#[cfg(test)]
mod tests;

use anyhow::{ensure, Context, Result};
use opencoder_core::harness::{Harness, HarnessRuntime};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

pub const HOME_MOUNT: &str = "/run/opencoder-codex/home";
pub const LAUNCH_MOUNT: &str = "/run/opencoder-codex/launch.json";
const DEFAULT_BINARY: &str = "/usr/bin/codex";
const GUEST_HOME: &str = "/tmp/codex-user";
const GUEST_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const INHERITED: &[&str] = &[
    "HOME",
    "CODEX_HOME",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "CODEX_SOCKS5_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

#[derive(Clone)]
pub struct Launch {
    pub runtime: HarnessRuntime,
    pub home: PathBuf,
}

/// Resolve the selected Agent's private profile against the node configuration.
/// Rootfs executables are guest paths: host wrappers are never run as a fallback.
pub fn resolve(
    config: &opencoder_core::Config,
    agent: &str,
    rootfs: &Path,
) -> Result<Option<Launch>> {
    opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
        if opencoder_core::harness::agent_harness(agent) != Harness::Codex {
            return Ok(None);
        }
        let mut runtime = HarnessRuntime {
            harness: Harness::Codex,
            ..Default::default()
        };
        opencoder_core::harness::pin_agent_settings(&mut runtime, config, agent)
            .map_err(anyhow::Error::msg)?;
        let mut inherited: BTreeMap<String, String> = INHERITED
            .iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| ((*key).to_string(), value))
            })
            .collect();
        // Service managers may omit HOME. Match the CLI's system-user home
        // discovery instead of requiring a new daemon environment variable.
        if let Some(home) = dirs::home_dir() {
            inherited
                .entry("HOME".into())
                .or_insert_with(|| home.display().to_string());
        }
        let home = credential_home(&runtime.envs, &inherited)?
            .canonicalize()
            .context("Codex home unavailable on execution node; log in or configure CODEX_HOME")?;
        ensure!(
            home.is_dir() && home.parent().is_some(),
            "Codex home must be a directory below the filesystem root"
        );
        runtime = guest_runtime(runtime, &inherited)?;
        let executable = runtime
            .codex
            .as_ref()
            .and_then(|c| c.executable.as_deref())
            .unwrap();
        validate_executable(rootfs, executable)?;
        Ok(Some(Launch { runtime, home }))
    })
}

fn credential_home(
    explicit: &BTreeMap<String, String>,
    inherited: &BTreeMap<String, String>,
) -> Result<PathBuf> {
    let get = |key: &str| explicit.get(key).or_else(|| inherited.get(key));
    let home = if let Some(value) = get("CODEX_HOME") {
        ensure!(!value.trim().is_empty(), "CODEX_HOME must not be empty");
        PathBuf::from(value)
    } else {
        let value = get("HOME")
            .filter(|v| !v.trim().is_empty())
            .context("Codex credentials need CODEX_HOME or HOME on the execution node")?;
        PathBuf::from(value).join(".codex")
    };
    ensure!(
        home.is_absolute(),
        "Codex credential home must be absolute for runc"
    );
    Ok(home)
}

fn guest_runtime(
    mut runtime: HarnessRuntime,
    inherited: &BTreeMap<String, String>,
) -> Result<HarnessRuntime> {
    for key in INHERITED {
        if let Some(value) = inherited.get(*key) {
            runtime
                .envs
                .entry((*key).into())
                .or_insert_with(|| value.clone());
        }
    }
    runtime.envs.insert("CODEX_HOME".into(), HOME_MOUNT.into());
    runtime.envs.insert("HOME".into(), GUEST_HOME.into());
    runtime
        .envs
        .entry("PATH".into())
        .or_insert_with(|| GUEST_PATH.into());
    let settings = runtime.codex.get_or_insert_with(Default::default);
    settings
        .executable
        .get_or_insert_with(|| DEFAULT_BINARY.into());
    // OCI owns the sandbox; avoid requiring a second nested sandbox or an
    // interactive approval channel. Explicit profile policies remain intact.
    settings
        .sandbox_mode
        .get_or_insert_with(|| "danger-full-access".into());
    settings
        .approval_policy
        .get_or_insert_with(|| "never".into());
    settings.envs = runtime.envs.clone();
    settings.validate().map_err(anyhow::Error::msg)?;
    Ok(runtime)
}

/// Validate paths before touching the host filesystem. Provisioned executable
/// and mountpoint parents must be real directories, never host-followed links.
pub(super) fn guest_path(rootfs: &Path, guest: &str) -> Result<PathBuf> {
    let path = Path::new(guest);
    ensure!(path.is_absolute(), "Codex guest path must be absolute");
    let mut target = rootfs.to_path_buf();
    for component in path.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => target.push(name),
            _ => anyhow::bail!("invalid Codex guest path"),
        }
        match std::fs::symlink_metadata(&target) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "Codex rootfs paths must not contain symlinks"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(target)
}

fn validate_executable(rootfs: &Path, executable: &str) -> Result<()> {
    ensure!(
        !["/tmp", "/proc", "/dev", "/run", "/workspace"]
            .iter()
            .any(|prefix| Path::new(executable).starts_with(prefix)),
        "Codex executable must be installed in the read-only rootfs"
    );
    let path = guest_path(rootfs, executable)?;
    let metadata = path.metadata().with_context(|| format!(
        "Codex executable missing in rootfs at {}; provision it with prepare-dag-rootfs.sh --codex", path.display()))?;
    ensure!(metadata.is_file(), "Codex rootfs executable must be a file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "Codex rootfs file is not executable"
        );
    }
    Ok(())
}

/// Called inside the container. A Codex card without its private launch file
/// must fail instead of reverting to container defaults or another account.
pub fn load_runtime(selected: Harness) -> Result<Option<HarnessRuntime>> {
    if selected != Harness::Codex {
        return Ok(None);
    }
    let runtime: HarnessRuntime = serde_json::from_slice(
        &std::fs::read(LAUNCH_MOUNT).context("Codex sandbox launch settings missing")?,
    )?;
    ensure!(
        runtime.harness == Harness::Codex && runtime.codex.is_some(),
        "invalid Codex sandbox launch settings"
    );
    std::fs::create_dir_all(GUEST_HOME)?;
    Ok(Some(runtime))
}
