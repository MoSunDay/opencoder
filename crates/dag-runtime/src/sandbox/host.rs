//! Sandbox host mapping shared by both backends. Every DAG sandbox root is an
//! empty per-run anchor carrying a tmpfs `/`; the host's system directories,
//! their nested mounts, optional resolver files and the runners directory are
//! bound read-only over it. Pure functions over `host` (the node passes
//! `host_root()`, tests map a fixture directory).
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, VecDeque},
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

/// The node's filesystem root: the only host mapping production ever uses.
pub fn host_root() -> &'static Path {
    Path::new("/")
}
/// Guest path of the read-only `dag.runners_dir` bind.
pub const RUNNERS_MOUNT: &str = "/run/opencoder/bin";
pub const RUNNER: &str = "dag-runner";
pub const STEP_RUNNER: &str = "agent-step-runner";
/// Sandbox `PATH` of the init and of every step.
pub const GUEST_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
/// Host directories bound read-only at the same guest path, in mount order.
pub const HOST_DIRECTORIES: [&str; 6] = ["/bin", "/etc", "/lib", "/lib64", "/sbin", "/usr"];
/// Host files bound read-only over the directories above when present.
pub const HOST_FILES: [&str; 2] = ["/etc/hosts", "/etc/resolv.conf"];
/// Link hops tolerated while mapping a guest path, like the kernel's limit.
const MAX_LINKS: usize = 40;

/// Anchor of the sandbox root inside the run's bundle: an empty directory the
/// runtime mounts the tmpfs `/` on inside its own mount namespace. It is never
/// a bind source and nothing stays mounted on it on the host.
pub fn root_dir(run_root: &Path) -> PathBuf {
    run_root.join("bundle/root")
}

/// The host directory holding `dag-runner` and `agent-step-runner`: the
/// explicit `dag.runners_dir`, else `usr/bin` of the configured image
/// directory `dag.rootfs_dir` (the key platform image replacement rewrites).
pub(crate) fn runners_dir(config: &opencoder_core::Config) -> Result<PathBuf> {
    if let Some(runners) = config.dag.runners_dir.as_deref() {
        return Ok(runners.to_path_buf());
    }
    if let Some(image) = config.dag.rootfs_dir.as_deref() {
        if !image.as_os_str().is_empty() {
            return Ok(image.join("usr/bin"));
        }
    }
    bail!("DAG runners_dir is required")
}

/// The configured runners directory, verified to hold this node's own runner
/// builds; the sandbox binds it read-only at `RUNNERS_MOUNT`.
pub(crate) fn runners_source(config: &opencoder_core::Config) -> Result<PathBuf> {
    let path = runners_dir(config)?;
    super::run::validate_directory(&path)?;
    let expected = serde_json::to_value(opencoder_core::version::build_info())?;
    for binary in [RUNNER, STEP_RUNNER] {
        let runner = path.join(binary);
        let metadata = std::fs::symlink_metadata(&runner)
            .with_context(|| format!("runners {binary} is missing"))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "runners {binary} must be a regular file"
        );
        let info = std::process::Command::new(&runner)
            .arg("--build-info")
            .output()
            .with_context(|| format!("read runners {binary} build metadata"))?;
        ensure!(
            info.status.success() && info.stdout.len() <= 8192,
            "runners {binary} build metadata unavailable"
        );
        let actual: Value = serde_json::from_slice(&info.stdout)
            .with_context(|| format!("invalid runners {binary} build metadata"))?;
        ensure!(
            actual == expected,
            "runners {binary} version does not match this execution node"
        );
    }
    Ok(path.to_path_buf())
}

/// OCI mounts of the sandbox root: the tmpfs `/` first, then the host system
/// directories with their nested mounts, the resolver files that exist and the
/// runners directory, every bind read-only and non-recursive.
pub(crate) fn mounts(mountinfo: &str, host: &Path, runners: &Path) -> Result<Vec<Value>> {
    let mut entries = vec![json!({"destination":"/","type":"tmpfs","source":"tmpfs",
        "options":["nosuid","nodev","noexec","mode=755","size=64m"]})];
    let mut seen = BTreeSet::new();
    for directory in HOST_DIRECTORIES {
        let source = directory_source(host, directory)?;
        push(&mut entries, &mut seen, &source, directory)?;
        for nested in super::run::mounts::nested_mounts(mountinfo, &source)? {
            let guest = Path::new(directory).join(nested.strip_prefix(&source)?);
            push(
                &mut entries,
                &mut seen,
                &nested,
                guest.to_str().context("host mount path must be UTF-8")?,
            )?;
        }
    }
    for file in HOST_FILES {
        if let Some(source) = file_source(host, file)? {
            push(&mut entries, &mut seen, &source, file)?;
        }
    }
    push(&mut entries, &mut seen, runners, RUNNERS_MOUNT)?;
    Ok(entries)
}

fn push(
    entries: &mut Vec<Value>,
    seen: &mut BTreeSet<String>,
    source: &Path,
    guest: &str,
) -> Result<()> {
    if !seen.insert(guest.to_owned()) {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("{guest} is missing in the sandbox host mapping"))?;
    ensure!(
        metadata.is_dir() || metadata.is_file(),
        "{guest} must be a directory or regular file in the sandbox host mapping"
    );
    if metadata.is_dir() {
        super::run::validate_directory(source)?;
    }
    entries.push(json!({"type":"bind","source":source,"destination":guest,
        "options":["bind","ro","nosuid","nodev"]}));
    Ok(())
}

/// Real host directory behind a mapped guest directory. A merged-usr link
/// (`/bin -> usr/bin`) is bound at its guest path through its target: a tmpfs
/// root has no links and the interpreters of host binaries need them.
fn directory_source(host: &Path, guest: &str) -> Result<PathBuf> {
    let path = host.join(guest.trim_start_matches('/'));
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("{guest} is missing in the sandbox host mapping"))?;
    let source = if metadata.file_type().is_symlink() {
        path.canonicalize()?
    } else {
        path
    };
    ensure!(
        source.is_dir(),
        "{guest} must be a directory in the sandbox host mapping"
    );
    ensure!(
        source.starts_with(host.canonicalize()?),
        "{guest} links outside the sandbox host root"
    );
    Ok(source)
}

/// Real host file behind an optional mapped guest file; `None` when absent.
fn file_source(host: &Path, guest: &str) -> Result<Option<PathBuf>> {
    let path = host.join(guest.trim_start_matches('/'));
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context(format!("inspect {guest}")),
    };
    let source = if metadata.file_type().is_symlink() {
        path.canonicalize()?
    } else {
        path
    };
    ensure!(
        source.is_file(),
        "{guest} must be a regular file in the sandbox host mapping"
    );
    Ok(Some(source))
}

/// Host file behind a guest path, following links exactly as the sandbox
/// would: a guest path below `RUNNERS_MOUNT` names the configured `runners`
/// directory, every other one the host root; an absolute link target is a
/// guest path too, so the resolved path has to stay inside the mapping.
pub(crate) fn host_path(host: &Path, runners: Option<&Path>, guest: &str) -> Result<PathBuf> {
    let host = host
        .canonicalize()
        .context("sandbox host root unavailable")?;
    let behind = |resolved: &Path| -> Result<PathBuf> {
        let path = Path::new("/").join(resolved);
        match (path.strip_prefix(RUNNERS_MOUNT), runners) {
            (Ok(rest), Some(runners)) => Ok(runners.join(rest)),
            (Ok(_), None) => bail!("DAG runners_dir is required for {guest}"),
            (Err(_), _) => Ok(host.join(resolved)),
        }
    };
    let mut pending = guest_components(guest)?;
    let mut resolved = PathBuf::new();
    let mut links = 0;
    while let Some(step) = pending.pop_front() {
        match step {
            Step::Parent => {
                resolved.pop();
                continue;
            }
            Step::Name(name) => resolved.push(name),
        }
        let path = behind(&resolved)?;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                links += 1;
                ensure!(
                    links <= MAX_LINKS,
                    "{guest}: too many links in the sandbox host mapping"
                );
                let link = std::fs::read_link(&path)?;
                let mut hops = guest_components(&link.to_string_lossy())?;
                if link.is_absolute() {
                    resolved.clear();
                } else {
                    resolved.pop();
                }
                hops.extend(pending);
                pending = hops;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context(format!("inspect {guest}")),
        }
    }
    let guest_path = Path::new("/").join(&resolved);
    let mapped = HOST_DIRECTORIES
        .iter()
        .any(|directory| guest_path.starts_with(directory))
        || guest_path.starts_with(RUNNERS_MOUNT);
    ensure!(mapped, "{guest} is missing in the sandbox host mapping");
    behind(&resolved)
}

/// Guest path components in kernel order; link targets may contain `..`.
fn guest_components(guest: &str) -> Result<VecDeque<Step>> {
    let mut steps = VecDeque::new();
    for component in Path::new(guest).components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => steps.push_back(Step::Name(name.to_owned())),
            Component::ParentDir => steps.push_back(Step::Parent),
            _ => bail!("invalid sandbox guest path: {guest}"),
        }
    }
    Ok(steps)
}

enum Step {
    Name(OsString),
    Parent,
}

/// A sandbox executable has to be a real executable file of the host mapping
/// (the runners directory included); writable or per-run guest paths are
/// refused.
pub(crate) fn validate_executable(
    host: &Path,
    runners: Option<&Path>,
    executable: &str,
) -> Result<()> {
    ensure!(
        Path::new(executable).is_absolute()
            && ![
                "/tmp",
                "/proc",
                "/dev",
                "/run/opencoder-private",
                "/workspace"
            ]
            .iter()
            .any(|prefix| Path::new(executable).starts_with(prefix)),
        "Codex executable must be installed in a read-only host mapping"
    );
    let path = host_path(host, runners, executable)?;
    let metadata = std::fs::symlink_metadata(&path).with_context(|| {
        format!("Codex executable missing on the host at {executable}; install it under /usr or dag.runners_dir")
    })?;
    ensure!(
        metadata.is_file(),
        "Codex host executable must be a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "Codex host executable is not executable"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runners_directory_prefers_the_explicit_key_then_the_image_usr_bin() {
        let mut config = opencoder_core::Config::default();
        assert_eq!(
            runners_dir(&config).unwrap_err().to_string(),
            "DAG runners_dir is required"
        );
        config.dag.rootfs_dir = Some(PathBuf::from("/images/current"));
        assert_eq!(
            runners_dir(&config).unwrap(),
            Path::new("/images/current/usr/bin")
        );
        config.dag.runners_dir = Some(PathBuf::from("/opt/runners"));
        assert_eq!(runners_dir(&config).unwrap(), Path::new("/opt/runners"));
    }

    /// A host root with the mapped directories and a runners directory whose
    /// `codex` links back into the host mapping, like a wrapper would.
    fn mapping() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("host");
        std::fs::create_dir_all(host.join("usr/bin")).unwrap();
        let runners = temp.path().join("runners");
        std::fs::create_dir_all(&runners).unwrap();
        let binary = host.join("usr/bin/codex");
        std::fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(runners.join("codex"), "#!/bin/sh\nexit 0\n").unwrap();
        std::os::unix::fs::symlink("/usr/bin/codex", runners.join("wrapper")).unwrap();
        (temp, host, runners)
    }

    #[test]
    fn guest_paths_below_the_runners_mount_name_the_runners_directory() {
        let (_temp, host, runners) = mapping();
        assert_eq!(
            host_path(&host, Some(&runners), "/run/opencoder/bin/codex").unwrap(),
            runners.join("codex")
        );
        assert_eq!(
            host_path(&host, Some(&runners), "/run/opencoder/bin/sub/../codex").unwrap(),
            runners.join("codex")
        );
        // A link inside the runners directory is a guest path: it resolves
        // through the host mapping, as it does inside the sandbox.
        assert_eq!(
            host_path(&host, Some(&runners), "/run/opencoder/bin/wrapper").unwrap(),
            host.join("usr/bin/codex")
        );
        assert_eq!(
            host_path(&host, None, "/usr/bin/codex").unwrap(),
            host.join("usr/bin/codex")
        );
        assert_eq!(
            host_path(&host, None, "/run/opencoder/bin/codex")
                .unwrap_err()
                .to_string(),
            "DAG runners_dir is required for /run/opencoder/bin/codex"
        );
        assert!(host_path(&host, Some(&runners), "/opt/codex").is_err());
        assert!(host_path(&host, Some(&runners), "/run/opencoder").is_err());
    }

    #[test]
    fn executables_are_validated_through_the_runners_directory() {
        let (_temp, host, runners) = mapping();
        let error = validate_executable(&host, Some(&runners), "/run/opencoder/bin/codex")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not executable"), "{error}");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            runners.join("codex"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        validate_executable(&host, Some(&runners), "/run/opencoder/bin/codex").unwrap();
        let error = validate_executable(&host, None, "/run/opencoder/bin/codex")
            .unwrap_err()
            .to_string();
        assert!(error.contains("DAG runners_dir is required"), "{error}");
        let error = validate_executable(&host, Some(&runners), "/run/opencoder/bin/missing")
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("Codex executable missing on the host at /run/opencoder/bin/missing"),
            "{error}"
        );
        let error = validate_executable(&host, Some(&runners), "/workspace/codex")
            .unwrap_err()
            .to_string();
        assert!(error.contains("read-only host mapping"), "{error}");
    }
}
