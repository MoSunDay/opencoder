//! Config discovery + environment-overlay machinery.
//!
//! Extracted from `config.rs` so the main module stays under the line gate.
//! Thread-local isolation (`scoped_config_home`) keeps config discovery and env
//! overlays off the process-global environment, avoiding the `setenv`/`getenv`
//! UB that crashed parallel test runs.

use std::path::{Path, PathBuf};

use super::Config;

/// `true` when `s` looks like an environment-variable name (uppercase +
/// underscores/digits). Used by the `/model` menu to decide whether to wrap an
/// api-key value as `"{NAME}"` (preserving env-var indirection via
/// `resolve_env`) or store it verbatim.
pub fn looks_like_env_var(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// Thread-local override that redirects config discovery + env overlays away
/// from the process-global environment.
///
/// `std::env::set_var`/`remove_var` are thread-unsafe at the libc level: under
/// parallel test execution a concurrent `getenv` can observe a transiently
/// corrupt environ and crash the whole test binary (taking unrelated tests
/// with it). This thread-local lets a test isolate config discovery to a
/// tempdir on the *current thread only* — no process-env mutation, so no UB —
/// while production code (which never sets it) keeps reading the real env.
///
/// When set, [`config_candidates`] resolves every global candidate inside the
/// override dir, and [`env_get`] returns `None` for every name (so env overlays
/// like `OPENCODER_MODEL` / `OPENAI_API_KEY` never leak in from the host).
pub fn scoped_config_home(home: PathBuf) -> ScopedConfigHome {
    let prev = ISOLATION.with(|c| c.borrow_mut().replace(home));
    ScopedConfigHome { prev }
}

/// RAII guard restoring the prior isolation state on drop. Created by
/// [`scoped_config_home`]; drop unwinds the override even if a test panics.
pub struct ScopedConfigHome {
    prev: Option<PathBuf>,
}

impl Drop for ScopedConfigHome {
    fn drop(&mut self) {
        ISOLATION.with(|c| *c.borrow_mut() = self.prev.take());
    }
}

thread_local! {
    static ISOLATION: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The override dir when a test has installed [`scoped_config_home`].
fn isolated_home() -> Option<PathBuf> {
    ISOLATION.with(|c| c.borrow().clone())
}

/// Resolve the home dir for config discovery: the thread-local override when a
/// test set it, otherwise the real `dirs::home_dir()`.
fn config_home_dir() -> Option<PathBuf> {
    isolated_home().or_else(dirs::home_dir)
}

/// Resolve the XDG config dir: the thread-local override when a test set it
/// (mirrors the tests that pointed both `HOME` and `XDG_CONFIG_HOME` at one
/// tempdir), otherwise the real `dirs::config_dir()`.
fn config_xdg_dir() -> Option<PathBuf> {
    isolated_home().or_else(dirs::config_dir)
}

/// Read an env var, *unless* a test isolation override is active on this
/// thread — in which case return `None` so host env never contaminates the
/// isolated config under test.
pub(super) fn env_get(name: &str) -> Option<String> {
    if isolated_home().is_some() {
        None
    } else {
        std::env::var(name).ok()
    }
}

pub(super) fn config_candidates(working_dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![
        working_dir.join(".opencoder").join("config.json"),
        working_dir.join("opencoder.json"),
    ];
    if let Some(home) = config_home_dir() {
        // ~/.opencoder/ (this binary's own config home) — highest-priority global,
        // so `opencoder` runs directly from any directory with no project config.
        v.push(home.join(".opencoder").join("config.json"));
        v.push(home.join(".opencoder").join("opencoder.json"));
    }
    if let Some(cfg) = config_xdg_dir() {
        v.push(cfg.join("opencoder").join("config.json"));
    }
    v
}

pub(super) fn apply_env(cfg: &mut Config) {
    if let Some(b) = env_get("OPENAI_BASE_URL") {
        if !b.is_empty() {
            cfg.provider.base_url = b.trim_end_matches('/').to_string();
        }
    }
    if let Some(m) = env_get("OPENCODER_MODEL") {
        if !m.is_empty() {
            cfg.model = m;
        }
    }
    if let Some(m) = env_get("OPENCODER_SMALL_MODEL") {
        if !m.is_empty() {
            cfg.small_model = Some(m);
        }
    }
    if let Some(v) = env_get("OPENCODER_CONTEXT_LIMIT") {
        if let Ok(n) = v.parse::<u64>() {
            cfg.context_limit = Some(n);
        }
    }
    if let Some(raw) = env_get("OPENCODER_CACHE_SALT") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => cfg.cache_salt = Some(true),
            "false" | "0" | "no" => cfg.cache_salt = Some(false),
            _ => {}
        }
    }
    // Proxy overlay: explicit OPENCODER_PROXY wins, then ALL_PROXY. Only set
    // when the user has not already configured `network.proxy` directly.
    if cfg.network.proxy.is_none() {
        for var in ["OPENCODER_PROXY", "ALL_PROXY"] {
            if let Some(v) = env_get(var) {
                let t = v.trim();
                if !t.is_empty() {
                    cfg.network.proxy = Some(t.to_string());
                    break;
                }
            }
        }
    }
}

pub(super) fn resolve_env(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let name = &trimmed[1..trimmed.len() - 1];
        std::env::var(name).unwrap_or_default()
    } else {
        trimmed.to_string()
    }
}
