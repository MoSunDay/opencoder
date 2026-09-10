//! Freeze-on-accept distribution of DAG wasm modules from the shared pool.
//!
//! The pool (usually a read-only NFS mount at `dag.wasm_dir`) is the
//! mutable, server-side source of truth: publishes and rollbacks flip its
//! `current` pointer at any time. A node *pins* at accept time by copying
//! the current `wasm.bin` of every pool-known module the spec references
//! into `<workflow_root>/_modules/<token>` — the shared module library the
//! wasm executor resolves modules from. Later pool churn can therefore
//! never change what an already-accepted run executes.
//!
//! Semantics mirror `resources::pin` for agent resources: unconfigured →
//! silent no-op (out-of-band staging stays valid); unknown pool entry →
//! skip (out-of-band module); configured-but-corrupt export → hard error
//! (fail-closed).

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

use opencoder_dag::{DagSpec, StepKind};
use opencoder_dag_wasm as wasm_pool;

/// Pure collection: first whitespace token of every wasm step's command,
/// deduped, order-preserving.
pub(crate) fn module_tokens(spec: &DagSpec) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for step in &spec.steps {
        if let StepKind::Wasm { command, .. } = &step.kind {
            if let Some(first) = command.split_whitespace().next() {
                let token = first.to_string();
                if !tokens.contains(&token) {
                    tokens.push(token);
                }
            }
        }
    }
    tokens
}

/// Map a module token (e.g. "tool.wasm") to its pool name ("tool"):
/// `None` when it lacks a `.wasm` suffix, the suffix is the whole token,
/// or the stem fails [`wasm_pool::validate_name`] (nested or traversing
/// paths can never come from the flat pool — those stay out-of-band).
pub(crate) fn pool_name(token: &str) -> Option<String> {
    let stem = token.strip_suffix(".wasm")?;
    if stem.is_empty() {
        return None;
    }
    wasm_pool::validate_name(stem).ok()?;
    Some(stem.to_string())
}

/// Pin every pool-known module the spec references into
/// `<workflow_root>/_modules/<token>`.
///
/// - `config.dag.wasm_dir` unset → `Ok(())` no-op (out-of-band staging
///   stays valid; nothing is created).
/// - pool entry missing for a referenced name → skip silently
///   (out-of-band module).
/// - pool entry present but `wasm.bin` unreadable or its sha256 ≠ the
///   version meta → hard `Err`: a configured-but-corrupt export rejects
///   the accept (fail-closed).
pub(crate) fn pin(
    config: &opencoder_core::Config,
    spec: &DagSpec,
    workflow_root: &Path,
) -> Result<()> {
    let Some(source) = config.dag.wasm_dir.as_deref() else {
        return Ok(());
    };
    let modules = workflow_root.join("_modules");
    for token in module_tokens(spec) {
        let Some(name) = pool_name(&token) else {
            continue;
        };
        let Some(meta) = wasm_pool::read_pool_meta(source, &name) else {
            continue;
        };
        let bytes = read_verified(source, &name, meta.current)?;
        stage(
            &modules,
            &token,
            &wasm_pool::wasm_bin(source, &name, meta.current),
            &bytes,
        )?;
    }
    Ok(())
}

/// Read the pool's current `wasm.bin` and verify it against its version
/// meta digest — the fail-closed gate in front of every copy.
fn read_verified(source: &Path, name: &str, version: u32) -> Result<Vec<u8>> {
    let bin = wasm_pool::wasm_bin(source, name, version);
    let bytes = read_capped(&bin, wasm_pool::MAX_WASM_BYTES)
        .with_context(|| format!("wasm pool binary unavailable: {}", bin.display()))?;
    let meta = wasm_pool::read_version_meta(source, name, version)
        .with_context(|| format!("wasm pool version meta unavailable: {}", bin.display()))?;
    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(&meta.sha256) {
        bail!(
            "wasm pool binary digest mismatch at {}: expected {}, got {actual}",
            bin.display(),
            meta.sha256
        );
    }
    Ok(bytes)
}

/// Copy `source` to `<modules>/<token>` atomically: an identical existing
/// target is left untouched; otherwise copy into a `.tmp-<token>.<pid>`
/// sibling, fsync it, then rename over the target (replacing an older
/// frozen version) and best-effort fsync the parent — same durability
/// shape as `resources::pin`.
fn stage(modules: &Path, token: &str, source: &Path, verified: &[u8]) -> Result<()> {
    let target = modules.join(token);
    if let Ok(existing) = read_capped(&target, wasm_pool::MAX_WASM_BYTES) {
        if existing == verified {
            return Ok(());
        }
    }
    opencoder_core::share_fs::durable_create_dir_all(modules)?;
    let staging = modules.join(format!(".tmp-{token}.{}", std::process::id()));
    let _cleanup = Staging(staging.clone());
    std::fs::copy(source, &staging)?;
    std::fs::File::open(&staging)?.sync_all()?;
    std::fs::rename(&staging, &target)?;
    if let Ok(dir) = std::fs::File::open(modules) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Read at most `cap` bytes plus one (a longer file can never compare
/// equal to a pool-validated module, so truncation only forces a replace).
fn read_capped(path: &Path, cap: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(cap as u64 + 1)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Lowercase hex sha256 — the digest format the pool records in its
/// version metas.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// A failed copy never leaves a `.tmp-` file behind — mirrors the
// `Staging` guard in `resources.rs`.
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.exists() {
            let removed = if self.0.is_dir() {
                std::fs::remove_dir_all(&self.0)
            } else {
                std::fs::remove_file(&self.0)
            };
            if let Err(error) = removed {
                tracing::error!(path = %self.0.display(), %error, "remove wasm pin staging file");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_dag::{SandboxMode, StepSpec};

    /// Minimal valid wasm module: 8-byte header (`\0asm` + LE version 1).
    const HEADER: &[u8] = b"\0asm\x01\0\0\0";

    fn module(payload: &[u8]) -> Vec<u8> {
        [HEADER, payload].concat()
    }

    fn config_with_pool(dir: &Path) -> opencoder_core::Config {
        let mut config = opencoder_core::Config::default();
        config.dag.wasm_dir = Some(dir.to_path_buf());
        config
    }

    fn spec(commands: &[&str]) -> DagSpec {
        DagSpec {
            name: "test-workflow".into(),
            description: None,
            steps: commands
                .iter()
                .enumerate()
                .map(|(i, command)| StepSpec {
                    name: format!("step-{i}"),
                    depends_on: vec![],
                    kind: StepKind::Wasm {
                        command: command.to_string(),
                        sandbox: None,
                    },
                    timeout_secs: None,
                })
                .collect(),
        }
    }

    fn no_tmp_files(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .all(|entry| !entry.file_name().to_string_lossy().starts_with(".tmp-"))
            })
            .unwrap_or(true)
    }

    #[test]
    fn module_tokens_takes_first_wasm_token_deduped() {
        let spec = DagSpec {
            name: "wf".into(),
            description: None,
            steps: vec![
                StepSpec {
                    name: "agent-step".into(),
                    depends_on: vec![],
                    kind: StepKind::Agent {
                        prompt: "ignored".into(),
                        agent: None,
                        model: None,
                        how_append: None,
                    },
                    timeout_secs: None,
                },
                StepSpec {
                    name: "first".into(),
                    depends_on: vec![],
                    kind: StepKind::Wasm {
                        command: "tool.wasm --flag x".into(),
                        sandbox: None,
                    },
                    timeout_secs: None,
                },
                StepSpec {
                    name: "empty".into(),
                    depends_on: vec![],
                    kind: StepKind::Wasm {
                        command: "   ".into(),
                        sandbox: None,
                    },
                    timeout_secs: None,
                },
                StepSpec {
                    name: "duplicate".into(),
                    depends_on: vec![],
                    kind: StepKind::Wasm {
                        command: "  tool.wasm  again".into(),
                        sandbox: Some(SandboxMode::Runc),
                    },
                    timeout_secs: None,
                },
                StepSpec {
                    name: "second".into(),
                    depends_on: vec![],
                    kind: StepKind::Wasm {
                        command: "other.wasm".into(),
                        sandbox: None,
                    },
                    timeout_secs: None,
                },
            ],
        };
        assert_eq!(
            module_tokens(&spec),
            vec!["tool.wasm".to_string(), "other.wasm".to_string()]
        );
    }

    #[test]
    fn pool_name_maps_only_flat_wasm_tokens() {
        assert_eq!(pool_name("tool.wasm"), Some("tool".to_string()));
        assert_eq!(pool_name("my_tool-2.wasm"), Some("my_tool-2".to_string()));
        assert_eq!(pool_name("tool"), None);
        assert_eq!(pool_name("tool.bin"), None);
        assert_eq!(pool_name(".wasm"), None);
        assert_eq!(pool_name("../x.wasm"), None);
        assert_eq!(pool_name("build/out.wasm"), None);
    }

    #[test]
    fn pin_is_a_no_op_without_a_configured_pool() {
        let workflow = tempfile::tempdir().unwrap();
        pin(
            &opencoder_core::Config::default(),
            &spec(&["tool.wasm"]),
            workflow.path(),
        )
        .unwrap();
        assert!(!workflow.path().join("_modules").exists());
    }

    #[test]
    fn pin_skips_modules_unknown_to_the_pool() {
        let pool = tempfile::tempdir().unwrap();
        let workflow = tempfile::tempdir().unwrap();
        pin(
            &config_with_pool(pool.path()),
            &spec(&["ghost.wasm", "build/nested.wasm"]),
            workflow.path(),
        )
        .unwrap();
        // Out-of-band modules stage nothing — not even the directory.
        assert!(!workflow.path().join("_modules").exists());
    }

    #[test]
    fn pin_freezes_the_pool_current_and_replaces_on_new_accept() {
        let pool = tempfile::tempdir().unwrap();
        let workflow = tempfile::tempdir().unwrap();
        // One realistic wat-compiled module for the first version.
        let v1 = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
        assert_eq!(
            wasm_pool::save_wasm_version(pool.path(), "tool", "first", &v1).unwrap(),
            1
        );
        pin(
            &config_with_pool(pool.path()),
            &spec(&["tool.wasm --flag"]),
            workflow.path(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(workflow.path().join("_modules/tool.wasm")).unwrap(),
            v1
        );

        // Pool churn after the accept: rollback (pointer stays) then a new
        // publish flips `current` — the NEXT accept freezes the new bytes.
        wasm_pool::rollback_wasm(pool.path(), "tool", 1).unwrap();
        let v2 = module(b"second");
        assert_eq!(
            wasm_pool::save_wasm_version(pool.path(), "tool", "second", &v2).unwrap(),
            2
        );
        pin(
            &config_with_pool(pool.path()),
            &spec(&["tool.wasm"]),
            workflow.path(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(workflow.path().join("_modules/tool.wasm")).unwrap(),
            v2
        );

        // Idempotent re-pin of the same current: still Ok, content equal,
        // and no `.tmp-` leftovers either way.
        pin(
            &config_with_pool(pool.path()),
            &spec(&["tool.wasm"]),
            workflow.path(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(workflow.path().join("_modules/tool.wasm")).unwrap(),
            v2
        );
        assert!(no_tmp_files(&workflow.path().join("_modules")));
    }

    #[test]
    fn tampered_pool_binary_fails_closed_and_stages_nothing() {
        let pool = tempfile::tempdir().unwrap();
        let workflow = tempfile::tempdir().unwrap();
        assert_eq!(
            wasm_pool::save_wasm_version(pool.path(), "tool", "first", &module(b"one")).unwrap(),
            1
        );
        std::fs::write(
            wasm_pool::wasm_bin(pool.path(), "tool", 1),
            module(b"tampered"),
        )
        .unwrap();
        let error = pin(
            &config_with_pool(pool.path()),
            &spec(&["tool.wasm"]),
            workflow.path(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("digest mismatch"), "{error:#}");
        let modules = workflow.path().join("_modules");
        assert!(!modules.exists() || no_tmp_files(&modules));
    }
}
