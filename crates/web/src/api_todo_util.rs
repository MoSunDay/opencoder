//! Shared plumbing for the `/api/todo/*` family (share-tree env / tool /
//! template management + workflow runs): the JSON error-response helpers in
//! the same shape as `api_envs`, share-root resolution (`Config` + effective
//! share dir, created on demand), the millisecond clock, and the `v<n>`
//! version-name helpers backing template versioning. Pure functions only.

use std::path::{Path, PathBuf};

use anyhow::Context;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use opencoder_core::Config;

pub fn error_400(msg: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

pub fn error_404(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

pub fn error_409(msg: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

pub fn error_500(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

/// Load the config for `workdir` and resolve the effective share root
/// (override → env → `config.agent.share_dir` → `~/.opencoder/share`),
/// creating the root when absent. Every `/api/todo/*` handler starts here so
/// a missing mount fails loudly (500) instead of silently writing elsewhere.
pub async fn share_root(workdir: &Path) -> anyhow::Result<(Config, PathBuf)> {
    let config = Config::load(workdir)?;
    let root = opencoder_core::share_fs::effective_share_dir(Some(&config))
        .context("share root unresolved")?;
    tokio::fs::create_dir_all(&root)
        .await
        .with_context(|| format!("create share root {}", root.display()))?;
    Ok((config, root))
}

/// Millisecond wall clock — the same source the todos persistence layer
/// stamps records with, so template metadata ages comparably.
pub fn now_ms() -> i64 {
    opencoder_core::message::now_ms()
}

/// Template version directory names are exactly `v` + ASCII digits (`v1`).
pub fn is_version(name: &str) -> bool {
    match name.strip_prefix('v') {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Next free version given the existing ones: `v{max+1}`, minimum `v1`.
/// Non-`v<n>` entries are ignored; absurd digit runs fail `parse` and drop out.
pub fn next_version(versions: &[String]) -> String {
    let max = versions
        .iter()
        .filter(|v| is_version(v))
        .filter_map(|v| v[1..].parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("v{}", max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_helpers_match_v_pattern_only() {
        assert!(is_version("v1"));
        assert!(is_version("v123"));
        assert!(!is_version(""));
        assert!(!is_version("v"));
        assert!(!is_version("1"));
        assert!(!is_version("v1x"));
        assert!(!is_version("active"));
    }

    #[test]
    fn next_version_increments_past_the_max() {
        assert_eq!(next_version(&[]), "v1");
        assert_eq!(next_version(&["v1".into()]), "v2");
        assert_eq!(
            next_version(&["v1".into(), "v9".into(), "v2".into()]),
            "v10"
        );
        assert_eq!(next_version(&["active".into()]), "v1");
    }
}
