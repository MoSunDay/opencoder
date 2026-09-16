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
use serde_json::{json, Value};

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

/// Template retention cap: a template keeps only its most recent
/// `MAX_TEMPLATE_VERSIONS` versions. `current` is never pruned and always
/// occupies one of the slots, so a single-version template counts 1/10.
pub const MAX_TEMPLATE_VERSIONS: usize = 10;

/// Oldest-first pruning list for the `versions` array. Given the array and
/// the version that is (or is about to become) `current`, return the version
/// names to drop so at most `MAX_TEMPLATE_VERSIONS` entries remain. `current`
/// is never dropped; when it is absent from the array the newest 10 by age
/// survive. Age is `created_at` (missing = oldest, i.e. 0) with the array
/// index as tiebreaker, and survivors keep their array order.
pub fn prunable_versions(versions: &[Value], current: Option<&str>) -> Vec<String> {
    if versions.len() <= MAX_TEMPLATE_VERSIONS {
        return Vec::new();
    }
    let mut ranked: Vec<(i64, usize, &str)> = versions
        .iter()
        .enumerate()
        .filter(|(_, v)| v.get("version").and_then(Value::as_str) != current)
        .map(|(i, v)| {
            (
                v.get("created_at").and_then(Value::as_i64).unwrap_or(0),
                i,
                v.get("version").and_then(Value::as_str).unwrap_or(""),
            )
        })
        .collect();
    ranked.sort();
    let drop = versions.len() - MAX_TEMPLATE_VERSIONS;
    ranked
        .iter()
        .take(drop)
        .map(|(_, _, name)| (*name).to_string())
        .collect()
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
    fn pruning_keeps_at_most_ten_with_current_protected() {
        let v = |n: &str, at: i64| json!({"version": n, "created_at": at});
        // At or below the cap nothing is pruned — a lone current counts 1/10.
        let versions: Vec<Value> = (1..=10).map(|i| v(&format!("v{i}"), i)).collect();
        assert!(prunable_versions(&versions, Some("v10")).is_empty());
        assert!(prunable_versions(&[v("v1", 1)], Some("v1")).is_empty());

        // Above the cap the oldest non-current entries go, in age order.
        let versions: Vec<Value> = (1..=12).map(|i| v(&format!("v{i}"), i)).collect();
        assert_eq!(prunable_versions(&versions, Some("v12")), vec!["v1", "v2"]);

        // A current pinned to an old version keeps its slot and shifts the cut.
        let versions: Vec<Value> = (1..=11).map(|i| v(&format!("v{i}"), i)).collect();
        assert_eq!(prunable_versions(&versions, Some("v1")), vec!["v2"]);
    }

    #[test]
    fn pruning_orders_by_created_at_and_tolerates_missing_fields() {
        let v = |n: &str, at: i64| json!({"version": n, "created_at": at});
        // Out-of-order array: age decides, not position.
        let versions: Vec<Value> = [7i64, 8, 9, 10, 11, 12, 13, 14, 15, 1, 16]
            .iter()
            .enumerate()
            .map(|(i, at)| v(&format!("v{}", i + 1), *at))
            .collect();
        assert_eq!(prunable_versions(&versions, Some("v11")), vec!["v10"]);

        // Missing created_at ranks oldest (0); missing version name never
        // matches `current`, so it is prunable like any other entry.
        let head = vec![v("v1", 3), json!({"version": "v2"}), v("v3", 4)];
        let versions = {
            let mut all: Vec<Value> = (4..=12).map(|i| v(&format!("v{i}"), i)).collect();
            all.splice(0..0, head);
            all
        };
        assert_eq!(prunable_versions(&versions, Some("v12")), vec!["v2", "v1"]);
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
