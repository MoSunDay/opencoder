use anyhow::{ensure, Context, Result};
use opencoder_dag::{DagSpec, StepKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Pin {
    Binary {
        resource: String,
        version: u32,
        sha256: String,
    },
    Agent {
        name: String,
        sha256: String,
        dependencies_sha256: String,
    },
}

pub fn freeze(root: &Path, config: &opencoder_core::Config, spec: &DagSpec) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let manifest = root.join("resources.json");
    if manifest.exists() {
        let pins: BTreeMap<String, Pin> = serde_json::from_slice(&std::fs::read(&manifest)?)?;
        ensure!(
            pins.len() == spec.steps.len(),
            "frozen resource inventory changed"
        );
        for step in &spec.steps {
            match (
                step.kind.executable(),
                pins.get(&step.name)
                    .context("frozen resource pin missing")?,
            ) {
                (
                    StepKind::Binary { resource, .. },
                    Pin::Binary {
                        resource: frozen,
                        version,
                        sha256: digest,
                    },
                ) => {
                    ensure!(frozen == resource, "frozen binary resource changed");
                    let (_, explicit) = opencoder_dag_binary::parse_resource_token(resource)
                        .context("invalid frozen binary token")?;
                    ensure!(
                        *version > 0 && explicit.is_none_or(|expected| expected == *version),
                        "frozen binary version changed"
                    );
                    let bytes = read(&root.join(&step.name).join("meta/program"))?;
                    ensure!(sha256(&bytes) == *digest, "frozen binary digest mismatch");
                    opencoder_dag_binary::validate_host_architecture(&bytes)
                        .map_err(anyhow::Error::msg)?;
                }
                (
                    StepKind::Agent { agent, .. },
                    Pin::Agent {
                        name,
                        sha256: digest,
                        dependencies_sha256,
                    },
                ) => {
                    ensure!(
                        name == agent.as_deref().unwrap_or("act"),
                        "frozen Agent resource changed"
                    );
                    let bytes = read(&root.join(&step.name).join("meta/frozen-agent.json"))?;
                    ensure!(sha256(&bytes) == *digest, "frozen Agent digest mismatch");
                    ensure!(
                        opencoder_agents::snapshot::digest(&root.join("resources/agents"))?
                            == *dependencies_sha256,
                        "frozen Agent dependencies digest mismatch"
                    );
                }
                _ => anyhow::bail!("frozen resource kind changed"),
            }
        }
        return Ok(());
    }
    let mut pins = BTreeMap::new();
    let mut programs = BTreeMap::new();
    for step in &spec.steps {
        if let StepKind::Binary { resource, .. } = step.kind.executable() {
            let source = config
                .dag
                .binary_dir
                .as_deref()
                .context("binary resource pool is required")?;
            let (name, explicit) = opencoder_dag_binary::parse_resource_token(resource)
                .context("invalid binary resource token")?;
            let meta = opencoder_dag_binary::read_pool_meta(source, &name)
                .context("binary pool entry unavailable")?;
            let version = explicit.unwrap_or(meta.current);
            ensure!(
                version > 0 && meta.history.contains(&version),
                "binary resource version unavailable"
            );
            let version_meta = opencoder_dag_binary::read_version_meta(source, &name, version)
                .context("binary version metadata unavailable")?;
            ensure!(
                meta.name == name && version_meta.version == version,
                "binary resource metadata identity mismatch"
            );
            super::sandbox::run::validate_directory(&opencoder_dag_binary::version_dir(
                source, &name, version,
            ))?;
            let bytes = read(&opencoder_dag_binary::binary_bin(source, &name, version))?;
            opencoder_dag_binary::validate_host_architecture(&bytes).map_err(anyhow::Error::msg)?;
            let actual = sha256(&bytes);
            ensure!(
                actual == version_meta.sha256 && bytes.len() as u64 == version_meta.size_bytes,
                "binary resource digest or size mismatch"
            );
            pins.insert(
                step.name.clone(),
                Pin::Binary {
                    resource: resource.clone(),
                    version,
                    sha256: actual,
                },
            );
            programs.insert(step.name.clone(), bytes);
        }
    }
    for (step, bytes) in programs {
        let meta = root.join(step).join("meta");
        std::fs::create_dir_all(&meta)?;
        opencoder_core::atomic_write(&meta.join("program"), &bytes)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(meta.join("program"), std::fs::Permissions::from_mode(0o555))?;
    }
    let parent = root.parent().context("run parent missing")?;
    let run_id = root
        .file_name()
        .and_then(|name| name.to_str())
        .context("invalid run path")?;
    let names: Vec<_> = spec
        .steps
        .iter()
        .filter_map(|step| match step.kind.executable() {
            StepKind::Agent { agent, .. } => Some(agent.clone().unwrap_or_else(|| "act".into())),
            _ => None,
        })
        .collect();
    let (agents, dependencies_sha256) = if names.is_empty() {
        (None, String::new())
    } else {
        let agents = opencoder_agents::snapshot::pin_selected(
            config.agent.agents_dir.as_deref(),
            &root.join("resources/agents"),
            Some(&names),
        )?;
        let digest = opencoder_agents::snapshot::digest(
            agents.as_deref().context("Agent snapshot missing")?,
        )?;
        (agents, digest)
    };
    opencoder_core::agent::scope::with_root_sync(agents, || {
        for step in &spec.steps {
            crate::exec::how_copy::freeze(parent, run_id, step)?;
            if let StepKind::Agent { agent, .. } = step.kind.executable() {
                let bytes = read(&root.join(&step.name).join("meta/frozen-agent.json"))?;
                pins.insert(
                    step.name.clone(),
                    Pin::Agent {
                        name: agent.as_deref().unwrap_or("act").into(),
                        sha256: sha256(&bytes),
                        dependencies_sha256: dependencies_sha256.clone(),
                    },
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })?;
    opencoder_core::atomic_write(&manifest, &serde_json::to_vec_pretty(&pins)?)?;
    Ok(())
}

pub fn execution_config(
    root: &Path,
    config: &opencoder_core::Config,
    spec: &DagSpec,
) -> Result<opencoder_core::Config> {
    let mut pinned = config.clone();
    pinned.agent.agents_dir = if spec
        .steps
        .iter()
        .any(|step| matches!(step.kind.executable(), StepKind::Agent { .. }))
    {
        let path = root.join("resources/agents");
        super::sandbox::run::validate_directory(&path)?;
        Some(path)
    } else {
        None
    };
    Ok(pinned)
}

pub fn frozen_resources(root: &Path) -> Result<Option<BTreeMap<String, serde_json::Value>>> {
    let bytes = match read(&root.join("resources.json")) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let pins: BTreeMap<String, Pin> = serde_json::from_slice(&bytes)?;
    pins.into_iter()
        .map(|(name, pin)| Ok((name, serde_json::to_value(pin)?)))
        .collect::<Result<BTreeMap<_, _>>>()
        .map(Some)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(file.metadata()?.is_file(), "binary must be a regular file");
    let mut bytes = Vec::new();
    file.take(opencoder_dag_binary::MAX_BINARY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= opencoder_dag_binary::MAX_BINARY_BYTES,
        "binary exceeds resource size limit"
    );
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
