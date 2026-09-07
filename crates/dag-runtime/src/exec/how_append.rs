//! `how_append` plumbing for DAG agent steps.
//!
//! A workflow author may declare `how_append` on an agent step. The value
//! is exposed to the step's session as `OPENCODER_HOW_APPEND` (forwarded
//! into every tool process via `SessionState::env_passthrough` →
//! `ToolContext::extra_env`), and after the step finishes successfully the
//! runtime appends it to the executing agent's shared prompt-pool
//! `how.md` as a NEW pool version — versions only grow, rollback is a
//! pointer flip, and every agent referencing the pool sees the append.

use opencoder_agents::write::{save_resource_version, VersionFile};

/// Session-visible env var carrying the workflow-declared append payload.
pub const HOW_APPEND_ENV: &str = "OPENCODER_HOW_APPEND";

/// The `how.md` section file inside a prompts pool version.
const HOW_FILE: &str = "how.md";

/// Env pairs for a step session; empty when the step declares nothing.
pub fn env_pairs(how_append: Option<&str>) -> Vec<(String, String)> {
    how_append
        .map(|text| (HOW_APPEND_ENV.to_string(), text.to_string()))
        .into_iter()
        .collect()
}

/// Resolve the prompts pool the agent actually reads: its reference card's
/// `current.prompt` (shared pools may carry any name), falling back to a
/// pool named after the agent (the `act` default for steps without an
/// `agent` field — the pool may not exist yet; the append creates it).
fn pool_name(agent: &str) -> String {
    opencoder_core::agent::read_agent_meta(agent)
        .and_then(|meta| meta.current.prompt)
        .unwrap_or_else(|| agent.to_string())
}

/// Append `delta` to the agent's pool `how.md` as a new version carrying
/// every file of the current version plus the updated section. Blank-safe:
/// the joiner is a blank line and both sides are trimmed.
pub fn append_to_how_md(agent: &str, delta: &str) -> Result<u32, String> {
    let pool = pool_name(agent);
    let mut files = current_pool_files(&pool)?;
    let existing = files
        .iter()
        .find(|f| f.rel_path == HOW_FILE)
        .map(|f| String::from_utf8_lossy(&f.bytes).into_owned())
        .unwrap_or_default();
    let mut next = existing.trim_end().to_string();
    if !next.is_empty() {
        next.push_str("\n\n");
    }
    next.push_str(delta.trim());
    let bytes = next.into_bytes();
    match files.iter_mut().find(|f| f.rel_path == HOW_FILE) {
        Some(file) => file.bytes = bytes,
        None => files.push(VersionFile {
            rel_path: HOW_FILE.into(),
            bytes,
        }),
    }
    save_resource_version("prompts", &pool, &files).map_err(|e| e.to_string())
}

/// Snapshot the pool's current version as `VersionFile`s (flat dir, sorted
/// for determinism); an absent pool or version yields an empty vec.
fn current_pool_files(pool: &str) -> Result<Vec<VersionFile>, String> {
    let Some(dir) = opencoder_core::agent::resource_current_version_dir("prompts", pool) else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let bytes = std::fs::read(dir.join(&name)).map_err(|e| format!("read {name}: {e}"))?;
            Ok(VersionFile {
                rel_path: name,
                bytes,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// The agents-root override is process-global; serialize the tests
    /// that flip it.
    static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

    fn scoped() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
        let tmp = tempfile::tempdir().unwrap();
        let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        opencoder_core::agent::set_agents_dir_override(Some(tmp.path().to_path_buf()));
        (tmp, guard)
    }

    #[test]
    fn env_pairs_empty_without_declaration() {
        assert!(env_pairs(None).is_empty());
        assert_eq!(
            env_pairs(Some("note")),
            vec![(HOW_APPEND_ENV.to_string(), "note".to_string())]
        );
    }

    #[test]
    fn append_preserves_siblings_and_grows_versions() {
        let (tmp, _guard) = scoped();
        let pool = tmp.path().join("prompts/probe");

        // Fresh pool (no card/current version): the append creates v1 with
        // just how.md.
        assert_eq!(append_to_how_md("probe", "first note").unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(pool.join("v1/how.md")).unwrap(),
            "first note"
        );

        // Append again: v2 carries the grown how.md.
        assert_eq!(append_to_how_md("probe", "second note").unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(pool.join("v2/how.md")).unwrap(),
            "first note\n\nsecond note"
        );
        opencoder_core::agent::set_agents_dir_override(None);
    }

    #[test]
    fn append_keeps_unrelated_section_files() {
        let (tmp, _guard) = scoped();
        // Seed v1 with soul.md only (no how.md).
        save_resource_version(
            "prompts",
            "seed",
            &[VersionFile {
                rel_path: "soul.md".into(),
                bytes: b"identity".to_vec(),
            }],
        )
        .unwrap();
        // The append must keep soul.md and add how.md in the new version.
        assert_eq!(append_to_how_md("seed", "note").unwrap(), 2);
        let v2 = tmp.path().join("prompts/seed/v2");
        assert_eq!(
            std::fs::read_to_string(v2.join("soul.md")).unwrap(),
            "identity"
        );
        assert_eq!(std::fs::read_to_string(v2.join("how.md")).unwrap(), "note");
        opencoder_core::agent::set_agents_dir_override(None);
    }
}
