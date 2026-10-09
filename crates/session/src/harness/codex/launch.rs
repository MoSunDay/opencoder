//! External launcher arguments and process lifecycle.
use anyhow::{Context, Result};
use opencoder_core::harness::CodexSettings;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

pub fn resolve_program(
    program: &str,
    envs: &BTreeMap<String, String>,
    workdir: &Path,
) -> Result<PathBuf> {
    let root = std::env::current_dir()?.join(workdir);
    let path = Path::new(program);
    let candidates = if path.is_absolute() || path.components().count() > 1 {
        vec![root.join(path)]
    } else {
        let paths = envs
            .get("PATH")
            .map(OsString::from)
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        std::env::split_paths(&paths)
            .map(|p| root.join(p).join(program))
            .collect()
    };
    candidates
        .into_iter()
        .find(|p| {
            p.metadata().is_ok_and(|m| {
                if !m.is_file() {
                    return false;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    true
                }
            })
        })
        .context("Codex startup script executable unavailable on execution node")
}

pub fn startup_program(
    settings: Option<&CodexSettings>,
    envs: &BTreeMap<String, String>,
    workdir: &Path,
) -> Result<Option<PathBuf>> {
    settings
        .and_then(|s| s.startup_script.first())
        .map(|p| resolve_program(p, envs, workdir))
        .transpose()
}

/// No shell splitting or interpolation: spaces and metacharacters remain literal.
pub fn prefix(settings: Option<&CodexSettings>, binary: &Path) -> Vec<OsString> {
    match settings.filter(|s| !s.startup_script.is_empty()) {
        Some(settings) => settings.startup_script[1..]
            .iter()
            .map(OsString::from)
            .chain(std::iter::once(binary.as_os_str().to_owned()))
            .collect(),
        None => Vec::new(),
    }
}

pub fn context(session: &crate::SessionState) -> String {
    serde_json::json!({
        "schema_version": 1,
        "session_id": session.id,
        "agent": session.agent.name,
        "thread_id": session.harness.thread_id,
        "fork_from": session.harness.fork_from,
        "input_id": session.harness.last_input_id,
        "workdir": session.working_dir,
    })
    .to_string()
}
