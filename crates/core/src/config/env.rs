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

/// The binary's own config home (`~/.opencoder`): the directory that owns
/// [`primary_global_config_path`] *and* the domain config files
/// (`mcp.json` / `cli.json` / `skills.json`). Kept here so domain-file
/// discovery shares the exact home resolution (and therefore the
/// [`scoped_config_home`] test override) of `config.json`. `pub(crate)` so
/// sibling modules outside `config` (e.g. `agent::meta`'s agents root) share
/// the exact same home resolution instead of re-deriving it.
pub(crate) fn global_opencoder_home() -> Option<PathBuf> {
    config_home_dir().map(|home| home.join(".opencoder"))
}

/// Canonical global config file owned by this binary.
///
/// Kept here so production and tests share the same home-resolution rules;
/// [`scoped_config_home`] therefore isolates both discovery and first-run
/// creation without mutating process-wide environment variables.
pub(super) fn primary_global_config_path() -> Option<PathBuf> {
    global_opencoder_home().map(|home| home.join("config.json"))
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
pub(crate) fn env_get(name: &str) -> Option<String> {
    if isolated_home().is_some() {
        None
    } else {
        std::env::var(name).ok()
    }
}

pub(super) fn config_candidates(working_dir: &Path) -> Vec<PathBuf> {
    config_candidates_with(working_dir, super::envs::active_env().as_deref())
}

/// Candidate chain with an explicit env layer override. `Some(name)` inserts
/// `~/.opencoder/envs/<name>/config.json` between the project files and the
/// global home (project > env > ~/.opencoder > XDG); `None` is the base chain
/// — also what env capture snapshots run against, avoiding self-reference.
pub(super) fn config_candidates_with(working_dir: &Path, active: Option<&str>) -> Vec<PathBuf> {
    let mut v = vec![
        working_dir.join(".opencoder").join("config.json"),
        working_dir.join("opencoder.json"),
    ];
    if let (Some(home), Some(name)) = (config_home_dir(), active) {
        v.push(
            home.join(".opencoder")
                .join("envs")
                .join(name)
                .join("config.json"),
        );
    }
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
    // Model selection FIRST: `OPENCODER_MODEL` may switch the active provider,
    // and the `OPENAI_BASE_URL` handling below syncs the *now-current*
    // provider registry entry — applying base_url before the switch would
    // leave the newly-active entry with a stale base_url.
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
    if let Some(m) = env_get("OPENCODER_EMBEDDING_MODEL") {
        if !m.is_empty() {
            cfg.embedding_model = Some(m);
        }
    }
    if let Some(b) = env_get("OPENAI_BASE_URL") {
        if !b.is_empty() {
            let normalized = b.trim_end_matches('/').to_string();
            cfg.provider.base_url = normalized.clone();
            // Sync the active provider registry entry too (same derivation as
            // `Config::provider_id`): without this, a provider selected via
            // `OPENCODER_MODEL` would keep its stale file-level base_url and
            // silently ignore the env override at endpoint resolution.
            let pid = cfg
                .model
                .split_once('/')
                .map(|(p, _)| p)
                .unwrap_or("openai")
                .to_string();
            if let Some(entry) = cfg.providers.get_mut(&pid) {
                entry.base_url = normalized;
            }
        }
    }
    if let Some(v) = env_get("OPENCODER_CONTEXT_LIMIT") {
        match parse_context_limit(&v) {
            Some(n) => cfg.context_limit = Some(n),
            // Empty = not set by the user (some shells export empty strings);
            // a non-empty non-u64 value is a real typo worth surfacing once.
            None if !v.is_empty() => tracing::warn!(
                value = %v,
                "invalid OPENCODER_CONTEXT_LIMIT (expected a plain u64, e.g. `8192`); ignoring"
            ),
            None => {}
        }
    }
    if let Some(raw) = env_get("OPENCODER_CACHE_SALT") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => cfg.cache_salt = Some(true),
            "false" | "0" | "no" => cfg.cache_salt = Some(false),
            _ => {}
        }
    }
    // opencoder-team overlay: workspace root + turn budgets. Same strictness
    // as OPENCODER_CONTEXT_LIMIT — a non-numeric turn budget is warned about
    // rather than silently coerced.
    if let Some(v) = env_get("OPENCODER_TEAM_ROOT") {
        if !v.is_empty() {
            cfg.team_root = PathBuf::from(v);
        }
    }
    if let Some(v) = env_get("OPENCODER_TEAM_MAX_TURNS") {
        match parse_plain_usize(&v) {
            Some(n) => cfg.team_max_turns = n,
            None if !v.is_empty() => tracing::warn!(
                value = %v,
                "invalid OPENCODER_TEAM_MAX_TURNS (expected a plain usize, e.g. `8`); ignoring"
            ),
            None => {}
        }
    }
    if let Some(v) = env_get("OPENCODER_TEAM_MAX_SUB_TURNS") {
        match parse_plain_usize(&v) {
            Some(n) => cfg.team_max_sub_turns = n,
            None if !v.is_empty() => tracing::warn!(
                value = %v,
                "invalid OPENCODER_TEAM_MAX_SUB_TURNS (expected a plain usize, e.g. `3`); ignoring"
            ),
            None => {}
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

/// Parse an `OPENCODER_CONTEXT_LIMIT` value: a plain `u64` literal, nothing
/// else. Deliberately strict — no trimming, no exponents, no sign — so a
/// typo like `" 8192"` or `"1e5"` is reported (warned by [`apply_env`])
/// instead of being silently coerced into a wrong window size. Pure.
pub(super) fn parse_context_limit(raw: &str) -> Option<u64> {
    raw.parse::<u64>().ok()
}

/// Parse an `OPENCODER_TEAM_MAX_TURNS` / `OPENCODER_TEAM_MAX_SUB_TURNS`
/// value: a plain `usize` literal with the same strictness (no trimming,
/// no sign, no exponents) as [`parse_context_limit`]. Pure.
pub(super) fn parse_plain_usize(raw: &str) -> Option<usize> {
    raw.parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::{parse_context_limit, parse_plain_usize};

    #[test]
    fn parse_context_limit_accepts_plain_u64() {
        assert_eq!(parse_context_limit("8192"), Some(8192));
        assert_eq!(parse_context_limit("0"), Some(0));
        assert_eq!(
            parse_context_limit("18446744073709551615"),
            Some(u64::MAX),
            "u64::MAX literal is still a plain u64"
        );
    }

    #[test]
    fn parse_context_limit_rejects_garbage() {
        assert_eq!(parse_context_limit("abc"), None, "non-numeric");
        assert_eq!(parse_context_limit(""), None, "empty is not a number");
        assert_eq!(parse_context_limit("-1"), None, "negative");
        assert_eq!(parse_context_limit("1e5"), None, "exponent notation");
        assert_eq!(
            parse_context_limit(" 8192"),
            None,
            "leading space: no trimming"
        );
        assert_eq!(
            parse_context_limit("8192 "),
            None,
            "trailing space: no trimming"
        );
        assert_eq!(
            parse_context_limit("18446744073709551616"),
            None,
            "u64 overflow"
        );
    }

    #[test]
    fn parse_plain_usize_accepts_and_rejects_like_context_limit() {
        assert_eq!(parse_plain_usize("8"), Some(8));
        assert_eq!(parse_plain_usize("0"), Some(0));
        assert_eq!(
            parse_plain_usize("18446744073709551615"),
            Some(usize::MAX),
            "usize::MAX literal is still a plain usize"
        );
        assert_eq!(parse_plain_usize("abc"), None, "non-numeric");
        assert_eq!(parse_plain_usize(""), None, "empty is not a number");
        assert_eq!(parse_plain_usize("-1"), None, "negative");
        assert_eq!(parse_plain_usize(" 8"), None, "leading space: no trimming");
        assert_eq!(parse_plain_usize("8 "), None, "trailing space: no trimming");
    }
}
