//! Immutable managed startup programs and the minimal private paths they need.
//! The program and its files are host files of the sandbox host mapping (see
//! `host`); only private settings are snapshotted into the run root.
use crate::sandbox::host;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MANIFEST: &str = "/usr/share/opencoder/codex-startup.json";
#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

#[derive(Clone, Serialize, Deserialize)]
pub struct PrivateMount {
    pub source: PathBuf,
    pub destination: String,
    pub directory: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Managed {
    pub program: PathBuf,
    pub config: PathBuf,
    pub configuration: serde_json::Value,
    pub mounts: Vec<PrivateMount>,
}

/// `runners` is the configured runners directory (`None` when unset): the
/// program, its files and the Codex executable are guest paths.
pub fn resolve(host: &Path, runners: Option<&Path>, command: &[String]) -> Result<Option<Managed>> {
    let Some(program) = command.first() else {
        return Ok(None);
    };
    let manifest = host::host_path(host, runners, MANIFEST)?;
    let data = match std::fs::read(manifest) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let manifest: serde_json::Value = serde_json::from_slice(&data)?;
    if manifest["program"] != program.as_str() {
        return Ok(None);
    }
    ensure!(
        manifest["schema_version"] == 1,
        "managed Codex startup schema differs"
    );
    ensure!(
        command.len() == 3 && command[1] == "--config",
        "managed Codex startup requires --config path"
    );
    for (name, expected) in manifest["files"]
        .as_object()
        .context("managed startup files missing")?
    {
        // Files sit beside the program in the host mapping; the digest pins
        // the exact bytes the sandbox binds and executes.
        let source = Path::new(program)
            .parent()
            .context("startup parent missing")?
            .join(name);
        let source = source.to_str().context("startup path is not UTF-8")?;
        let mapped = host::host_path(host, runners, source)?;
        let metadata = std::fs::symlink_metadata(&mapped)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "managed startup file must be a regular file: {source}"
        );
        let digest = format!("{:x}", Sha256::digest(std::fs::read(&mapped)?));
        ensure!(
            expected.as_str() == Some(digest.as_str()),
            "managed startup program differs from its manifest digest"
        );
    }
    let config = PathBuf::from(&command[2]);
    real_path(&config, false)?;
    let configuration: serde_json::Value = serde_json::from_slice(&std::fs::read(&config)?)?;
    ensure!(
        configuration.get("state_dir").is_none(),
        "managed startup must derive state from deployment_config"
    );
    let mut mounts = vec![];
    for key in ["proxy_file", "deployment_config"] {
        let source = PathBuf::from(
            configuration[key]
                .as_str()
                .context("managed startup private file missing")?,
        );
        real_path(&source, false)?;
        mounts.push(private_mount(source, false));
    }
    let home = PathBuf::from(
        configuration["codex_home"]
            .as_str()
            .context("managed Codex home missing")?,
    );
    let sqlite = PathBuf::from(
        configuration["sqlite_home"]
            .as_str()
            .context("managed Codex SQLite home missing")?,
    );
    real_path(&home, true)?;
    ensure!(
        sqlite.starts_with(&home),
        "Codex SQLite home must be within the shared credential/session home"
    );
    let codex = configuration["codex"]
        .as_str()
        .context("managed Codex executable missing")?;
    host::validate_executable(host, runners, codex)?;
    ensure!(
        manifest["codex"] == codex,
        "managed Codex executable differs from the startup manifest"
    );
    let clients = PathBuf::from(
        configuration["client_config_home"]
            .as_str()
            .context("managed client config home missing")?,
    );
    connection(&clients.join("opencoder/ctl.json"), &mut mounts)?;
    if let Some(files) = configuration.get("connection_files") {
        for file in files
            .as_array()
            .context("managed startup connection_files must be an array")?
        {
            let path = file
                .as_str()
                .context("managed startup connection file must be a path string")?;
            connection(Path::new(path), &mut mounts)?;
        }
    }
    Ok(Some(Managed {
        program: program.into(),
        config,
        configuration,
        mounts,
    }))
}

fn connection(path: &Path, mounts: &mut Vec<PrivateMount>) -> Result<()> {
    real_path(path, false)?;
    let connection: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if let Some(token) = connection["token_file"].as_str() {
        let token = PathBuf::from(token);
        let token = if token.is_absolute() {
            token
        } else {
            path.parent().unwrap().join(token)
        };
        real_path(&token, false)?;
        mounts.push(private_mount(token, false));
    }
    mounts.push(private_mount(path.to_path_buf(), false));
    Ok(())
}

fn private_mount(source: PathBuf, directory: bool) -> PrivateMount {
    PrivateMount {
        destination: source.display().to_string(),
        source,
        directory,
    }
}

fn real_path(path: &Path, directory: bool) -> Result<()> {
    ensure!(path.is_absolute(), "managed startup paths must be absolute");
    ensure!(
        path.canonicalize()? == path,
        "managed startup path cannot alias a symlink"
    );
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        !meta.file_type().is_symlink()
            && if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            },
        "managed startup path has wrong file type"
    );
    Ok(())
}

impl Managed {
    /// Snapshot the private settings under the run root and bind each declared
    /// private file read-only at its host path; the runtime creates the
    /// mountpoints of files the host mapping does not already carry.
    pub fn prepare(
        &self,
        root: &Path,
        step: &str,
        mounts: &mut Vec<serde_json::Value>,
    ) -> Result<()> {
        let private = root.join("private/codex").join(step);
        let config = private.join("startup.json");
        opencoder_core::atomic_write(&config, &serde_json::to_vec(&self.configuration)?)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600))?;
        let config_mount = PrivateMount {
            source: config.clone(),
            destination: self.config.display().to_string(),
            directory: false,
        };
        for entry in self.mounts.iter().chain(std::iter::once(&config_mount)) {
            let duplicate = mounts
                .iter()
                .find(|mount| mount["destination"] == entry.destination);
            if let Some(mount) = duplicate {
                // Multiple steps use the same declaration. Snapshot bytes, not
                // paths under step names, determine whether a config is equal.
                let source = mount["source"]
                    .as_str()
                    .context("private mount source missing")?;
                ensure!(
                    std::fs::read(source)? == std::fs::read(&entry.source)?,
                    "managed private mount differs between steps"
                );
                continue;
            }
            mounts.push(serde_json::json!({"type":"bind","source":entry.source,"destination":entry.destination,
                "options":["bind","ro","nosuid","nodev","noexec"]}));
        }
        let mut frozen = self.clone();
        frozen.config = config;
        opencoder_core::atomic_write(
            &private.join("startup-binding.json"),
            &serde_json::to_vec(&frozen)?,
        )?;
        Ok(())
    }
}
