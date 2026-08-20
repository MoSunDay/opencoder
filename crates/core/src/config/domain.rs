//! Domain config files (`mcp.json` / `cli.json` / `skills.json` / `ap.json`).
//!
//! The three map-shaped domains (`mcp_servers`, `cli`, `skills`) plus the
//! scalar `autopilot` domain are hard-cut from `config.json`: they load
//! from — and save to — a dedicated domain file.
//! Lookup walks project first and a single effective file wins (it shadows
//! the others entirely — no per-key merge across files, unlike
//! `config.json` candidates):
//!
//! - project: `<working_dir>/.opencoder/<domain>.json`
//! - env: `<global_opencode_home>/envs/<name>/<domain>.json` (active env only)
//! - global: `<global_opencode_home>/<domain>.json` (the home behind
//!   [`super::env::primary_global_config_path`], so the `scoped_config_home`
//!   override applies; XDG dirs are NOT consulted for domain files).

use std::path::{Path, PathBuf};

use super::Config;

/// Domain key -> domain file name. Order defines the split/save routing order.
pub(crate) const DOMAIN_FILES: [(&str, &str); 4] = [
    ("mcp_servers", "mcp.json"),
    ("cli", "cli.json"),
    ("skills", "skills.json"),
    ("autopilot", "ap.json"),
];

/// Placeholder path piece for a non-domain key: never matches a real file, so
/// callers treat the result as "no domain file".
const NOT_A_DOMAIN_FILE: &str = "__not_a_domain__.json";

/// Resolve a top-level config key to its `(key, file)` table entry.
fn domain_entry(key: &str) -> Option<(&'static str, &'static str)> {
    DOMAIN_FILES.iter().find(|(k, _)| *k == key).copied()
}

/// `true` when `key` is routed to a dedicated domain file instead of
/// `config.json`.
pub(crate) fn is_domain_key(key: &str) -> bool {
    domain_entry(key).is_some()
}

/// The domain file name for `key` (`mcp_servers` -> `mcp.json`), or `None`.
pub(crate) fn domain_file_name(key: &str) -> Option<&'static str> {
    domain_entry(key).map(|(_, file)| file)
}

/// Project-scope domain file: `<working_dir>/.opencoder/<domain>.json`.
/// A non-domain key yields a never-existing path (see [`NOT_A_DOMAIN_FILE`]).
pub(crate) fn project_domain_path(working_dir: &Path, key: &str) -> PathBuf {
    let file = domain_file_name(key).unwrap_or(NOT_A_DOMAIN_FILE);
    working_dir.join(".opencoder").join(file)
}

/// Global-scope domain file: `<global_opencode_home>/<domain>.json`. `None`
/// when the key is not a domain key or the home directory is unresolvable.
pub(crate) fn global_domain_path(key: &str) -> Option<PathBuf> {
    let file = domain_file_name(key).unwrap_or(NOT_A_DOMAIN_FILE);
    super::env::global_opencode_home().map(|home| home.join(file))
}

/// Env-scope domain file: `~/.opencoder/envs/<name>/<domain>.json` while an
/// env is active; `None` when no env layer applies.
fn env_domain_path(active: Option<&str>, key: &str) -> Option<PathBuf> {
    let file = domain_file_name(key)?;
    super::envs::env_dir(active?).map(|dir| dir.join(file))
}

/// The single effective domain file: the project one if it exists, else the
/// active env's, else the global one if it exists, else `None` (nothing to
/// load). Non-domain keys (guarded by [`is_domain_key`]) resolve to no file.
/// With an explicit env layer (`None` = base chain, used by env capture to
/// avoid self-reference).
pub(crate) fn effective_path_with(
    working_dir: &Path,
    key: &str,
    active: Option<&str>,
) -> Option<PathBuf> {
    if !is_domain_key(key) {
        return None;
    }
    let project = project_domain_path(working_dir, key);
    if project.exists() {
        return Some(project);
    }
    if let Some(env) = env_domain_path(active, key) {
        if env.exists() {
            return Some(env);
        }
    }
    let global = global_domain_path(key)?;
    if global.exists() {
        Some(global)
    } else {
        None
    }
}

/// Write target for a domain patch: the project file if it already exists,
/// else (while an env is active) the env file — created on save so the base
/// global file stays pristine for deactivation, else the global one — created
/// on save. Falls back to the project path when no home resolves.
pub(crate) fn write_target(working_dir: &Path, key: &str) -> Option<PathBuf> {
    write_target_with(working_dir, key, super::envs::active_env().as_deref())
}

/// [`write_target`] with an explicit env layer.
pub(crate) fn write_target_with(
    working_dir: &Path,
    key: &str,
    active: Option<&str>,
) -> Option<PathBuf> {
    if !is_domain_key(key) {
        return None;
    }
    let project = project_domain_path(working_dir, key);
    if project.exists() {
        return Some(project);
    }
    if let Some(env) = env_domain_path(active, key) {
        // Whether or not the env file exists, it is the target from here on
        // (created on save); the base global file stays untouched.
        return Some(env);
    }
    // Whether or not a global file exists, it is the target from here on
    // (created on save); only an unresolvable home falls back to the project.
    global_domain_path(key).or(Some(project))
}

/// Read the effective domain file for `key`. Missing/empty -> `None`. Corrupt
/// or non-object JSON warns (mirroring how [`Config::load`] skips bad config
/// candidates) and is treated as absent — a bad domain file must not break
/// startup.
pub(crate) fn read_effective(working_dir: &Path, key: &str) -> Option<serde_json::Value> {
    read_effective_with(working_dir, key, super::envs::active_env().as_deref())
}

/// [`read_effective`] with an explicit env layer (`None` = base chain).
pub(crate) fn read_effective_with(
    working_dir: &Path,
    key: &str,
    active: Option<&str>,
) -> Option<serde_json::Value> {
    let path = effective_path_with(working_dir, key, active)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) if v.is_object() => Some(v),
        Ok(v) => {
            tracing::warn!(
                "domain file {} is valid JSON but not an object (got {}); ignoring",
                path.display(),
                json_kind(&v)
            );
            None
        }
        Err(e) => {
            tracing::warn!("domain file {} is corrupt: {e}; ignoring", path.display());
            None
        }
    }
}

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Read a domain file's current content as the pre-merge root: missing file
/// → `{}`; empty/whitespace file → `{}`; anything unparseable is corrupt
/// (error — callers refuse to overwrite). Shared by [`save_domain`] and
/// [`probe_mcp_conflict`] so the dry-run sees exactly what a save would.
fn read_root(target: &Path) -> anyhow::Result<serde_json::Value> {
    if !target.exists() {
        return Ok(serde_json::json!({}));
    }
    let raw = std::fs::read_to_string(target)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", target.display()))?;
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) => Ok(v),
        // An empty/whitespace-only file is an empty object (matches a
        // freshly-created file); anything unparseable is corrupt.
        Err(_) if raw.trim().is_empty() => Ok(serde_json::json!({})),
        Err(e) => anyhow::bail!(
            "domain file {} is corrupt: {e}; refusing to overwrite",
            target.display()
        ),
    }
}

/// Merge `patch` into the domain file for `key` (pretty-printed, parents
/// created). A `null` entry deletes that key from the file (the
/// [`super::merge::merge_json`] semantics); a whole-domain `null` patch
/// empties the file to `{}` instead of writing a literal `null` (see the
/// normalization below). An existing-but-corrupt target file refuses the
/// write (error) and is left byte-for-byte untouched, mirroring
/// `Config::save_to`. Returns the path written.
pub(crate) fn save_domain(
    working_dir: &Path,
    key: &str,
    patch: &serde_json::Value,
) -> anyhow::Result<PathBuf> {
    let target = write_target(working_dir, key)
        .ok_or_else(|| anyhow::anyhow!("no write target for domain key `{key}`"))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut root: serde_json::Value = read_root(&target)?;
    super::merge::merge_json(&mut root, patch);
    // A whole-domain `null` patch (e.g. `{"cli": null}` after `split_patch`)
    // must not write a literal 4-byte `null` file: `merge_json`'s fallthrough
    // branch clobbers `root` wholesale, which pollutes the user-visible file
    // and makes the *next* save start from `Null` and whole-replace again.
    // An empty object deletes every entry while keeping the file present (so
    // corrupt-refusal semantics keep applying); the same normalization
    // defensively rescues any other non-object result (e.g. a pre-existing
    // `null`/scalar file merged with a plain object patch).
    if !root.is_object() {
        root = serde_json::json!({});
    }
    // MCP name-collision guard (bug #14), checked on the merged map right
    // before serialization — a refused save leaves the file untouched. The
    // domain file's top level IS the server map (no `mcp_servers` envelope).
    if key == "mcp_servers" {
        if let Some(servers) = root.as_object() {
            if let Some((offending, existing)) = super::mcp_guard::mcp_name_collision(servers) {
                anyhow::bail!(
                    "{}",
                    super::mcp_guard::conflict_message(&offending, &existing)
                );
            }
        }
    }
    let pretty = serde_json::to_string_pretty(&root)?;
    std::fs::write(&target, pretty)?;
    Ok(target)
}

/// Dry-run of the mcp guard inside [`save_domain`]: would the merge a
/// `Config::save(working_dir, patch)` performs for `mcp_servers` land on a
/// name collision? No disk access beyond reading the current target; returns
/// the [`super::mcp_guard::conflict_message`] on a hit. Read/target errors
/// yield `None` — the subsequent save surfaces them itself. Non-mcp domain
/// keys and config.json remainders are out of scope (they never carry
/// `mcp_servers`).
pub(crate) fn probe_mcp_conflict(working_dir: &Path, patch: &serde_json::Value) -> Option<String> {
    let (_, domains) = split_patch(patch);
    domains
        .iter()
        .filter(|(key, _)| *key == "mcp_servers")
        .find_map(|(key, value)| {
            let target = write_target(working_dir, key)?;
            let mut merged = read_root(&target).ok()?;
            super::merge::merge_json(&mut merged, value);
            let servers = merged.as_object()?;
            let (offending, existing) = super::mcp_guard::mcp_name_collision(servers)?;
            Some(super::mcp_guard::conflict_message(&offending, &existing))
        })
}

/// Pure patch split: extract top-level domain keys (values passed through
/// verbatim, including `null` deletions) and return `(config_remainder,
/// domain_entries)` where the remainder is a shallow copy without them.
pub(crate) fn split_patch(
    patch: &serde_json::Value,
) -> (serde_json::Value, Vec<(&'static str, serde_json::Value)>) {
    let mut remainder = serde_json::Map::new();
    let mut domains: Vec<(&'static str, serde_json::Value)> = Vec::new();
    if let Some(obj) = patch.as_object() {
        for (k, v) in obj {
            match domain_entry(k) {
                Some((key, _)) => domains.push((key, v.clone())),
                None => {
                    remainder.insert(k.clone(), v.clone());
                }
            }
        }
    }
    (serde_json::Value::Object(remainder), domains)
}

/// Apply one domain file's parsed JSON onto `cfg` (omitted fields preserve
/// siblings — the loops previously lived in `merge_into`). The three map
/// domains apply entry by entry; `autopilot` merges the whole object (its
/// file body IS the config, there is no entry map). Non-object values and
/// unknown keys are ignored.
pub(crate) fn apply_domain(cfg: &mut Config, key: &str, value: &serde_json::Value) {
    let entries = match value.as_object() {
        Some(o) => o,
        None => return,
    };
    match key {
        "mcp_servers" => {
            for (name, sv) in entries {
                if let Some(sobj) = sv.as_object() {
                    let entry = cfg.mcp_servers.entry(name.clone()).or_default();
                    super::mcp::merge(entry, sobj);
                }
            }
        }
        "cli" => {
            for (name, cv) in entries {
                if let Some(cobj) = cv.as_object() {
                    let entry = cfg.cli.entry(name.clone()).or_default();
                    super::cli::merge(entry, cobj);
                }
            }
        }
        "skills" => {
            for (name, sv) in entries {
                if let Some(sobj) = sv.as_object() {
                    let entry = cfg.skills.entry(name.clone()).or_default();
                    super::skill::merge(entry, sobj);
                }
            }
        }
        // Not entry-shaped: ap.json's top level is the AutoPilotConfig body.
        "autopilot" => super::autopilot::merge(&mut cfg.autopilot, entries),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_key_table_maps_keys_to_files() {
        for key in ["mcp_servers", "cli", "skills", "autopilot"] {
            assert!(is_domain_key(key), "{key} must be a domain key");
        }
        for key in ["model", "theme", "providers", "keymap", ""] {
            assert!(!is_domain_key(key), "{key} must not be a domain key");
        }
        assert_eq!(domain_file_name("mcp_servers"), Some("mcp.json"));
        assert_eq!(domain_file_name("cli"), Some("cli.json"));
        assert_eq!(domain_file_name("skills"), Some("skills.json"));
        assert_eq!(domain_file_name("autopilot"), Some("ap.json"));
        assert_eq!(domain_file_name("model"), None);
    }

    #[test]
    fn project_domain_path_lives_under_opencoder_dir() {
        let p = project_domain_path(Path::new("/w"), "skills");
        assert_eq!(p, PathBuf::from("/w/.opencoder/skills.json"));
        // non-domain keys map to a never-existing placeholder path
        let foreign = project_domain_path(Path::new("/w"), "theme");
        assert_eq!(
            foreign,
            PathBuf::from("/w/.opencoder/__not_a_domain__.json")
        );
    }

    #[test]
    fn global_domain_path_honors_scoped_home() {
        // Pure path construction (no filesystem touch): the scoped override
        // must redirect the global candidate, exactly like config.json.
        let probe = PathBuf::from("/scoped-home-probe");
        let _guard = super::super::env::scoped_config_home(probe.clone());
        assert_eq!(
            global_domain_path("mcp_servers"),
            Some(probe.join(".opencoder").join("mcp.json"))
        );
    }

    #[test]
    fn split_patch_separates_domain_keys_verbatim() {
        let patch = serde_json::json!({
            "theme": "light",
            "model": "prov/model",
            "skills": { "review": { "enabled": true } },
            "mcp_servers": { "srv": { "enabled": false } },
            "cli": null,
            "autopilot": { "mode": "ap" }
        });
        let (remainder, domains) = split_patch(&patch);
        assert_eq!(
            remainder,
            serde_json::json!({ "theme": "light", "model": "prov/model" })
        );
        assert_eq!(domains.len(), 4, "all four domain keys extracted");
        // values pass through verbatim, nulls included (delete semantics)
        let by_key = |k: &str| {
            domains
                .iter()
                .find(|(dk, _)| *dk == k)
                .map(|(_, v)| v)
                .cloned()
        };
        assert_eq!(
            by_key("skills"),
            Some(serde_json::json!({ "review": { "enabled": true } }))
        );
        assert_eq!(by_key("cli"), Some(serde_json::Value::Null));
        assert_eq!(
            by_key("mcp_servers"),
            Some(serde_json::json!({ "srv": { "enabled": false } }))
        );
        assert_eq!(
            by_key("autopilot"),
            Some(serde_json::json!({ "mode": "ap" }))
        );
    }

    #[test]
    fn split_patch_empty_patch_yields_empty_split() {
        let (remainder, domains) = split_patch(&serde_json::json!({}));
        assert_eq!(remainder, serde_json::json!({}));
        assert!(domains.is_empty());
    }

    #[test]
    fn apply_domain_routes_entries_to_the_right_field() {
        let mut cfg = Config::default();
        apply_domain(
            &mut cfg,
            "mcp_servers",
            &serde_json::json!({ "srv": { "enabled": true, "command": "npx" } }),
        );
        apply_domain(
            &mut cfg,
            "cli",
            &serde_json::json!({ "git": { "enabled": true, "content": "c" } }),
        );
        apply_domain(
            &mut cfg,
            "skills",
            &serde_json::json!({ "review": { "enabled": true } }),
        );
        apply_domain(
            &mut cfg,
            "autopilot",
            &serde_json::json!({ "mode": "review", "max_iterations": 7 }),
        );
        assert_eq!(cfg.mcp_servers.len(), 1);
        assert_eq!(cfg.mcp_servers["srv"].command.as_deref(), Some("npx"));
        assert_eq!(cfg.cli.len(), 1);
        assert_eq!(cfg.cli["git"].content, "c");
        assert_eq!(cfg.enabled_skill_names(), vec!["review".to_string()]);
        assert_eq!(cfg.autopilot.mode, super::super::ApMode::Review);
        assert_eq!(cfg.autopilot.max_iterations, 7);
    }

    /// `autopilot` is whole-object merged: a later partial ap.json patch (or
    /// file) preserves omitted siblings, exactly like the map domains.
    #[test]
    fn apply_domain_autopilot_merges_whole_object_and_preserves_siblings() {
        let mut cfg = Config::default();
        apply_domain(
            &mut cfg,
            "autopilot",
            &serde_json::json!({ "mode": "ap", "max_iterations": 5 }),
        );
        apply_domain(
            &mut cfg,
            "autopilot",
            &serde_json::json!({ "max_iterations": 9 }),
        );
        assert_eq!(
            cfg.autopilot.mode,
            super::super::ApMode::Ap,
            "mode preserved"
        );
        assert_eq!(cfg.autopilot.max_iterations, 9);
        // non-object bodies apply nothing (mirrors map-domain leniency)
        apply_domain(&mut cfg, "autopilot", &serde_json::json!(null));
        assert_eq!(cfg.autopilot.mode, super::super::ApMode::Ap);
    }

    #[test]
    fn apply_domain_entry_merge_preserves_siblings_and_toggles() {
        let mut cfg = Config::default();
        apply_domain(
            &mut cfg,
            "skills",
            &serde_json::json!({ "a": { "enabled": true }, "b": { "enabled": true } }),
        );
        // second patch toggles only `a`; sibling `b` must survive
        apply_domain(
            &mut cfg,
            "skills",
            &serde_json::json!({ "a": { "enabled": false } }),
        );
        assert!(!cfg.skills["a"].enabled, "patch must toggle `a` off");
        assert!(cfg.skills["b"].enabled, "sibling `b` must survive");
        assert_eq!(cfg.enabled_skill_names(), vec!["b".to_string()]);
    }

    #[test]
    fn apply_domain_ignores_non_object_values_and_unknown_keys() {
        let mut cfg = Config::default();
        apply_domain(&mut cfg, "skills", &serde_json::json!(null));
        apply_domain(&mut cfg, "skills", &serde_json::json!([1, 2]));
        apply_domain(&mut cfg, "skills", &serde_json::json!("str"));
        apply_domain(&mut cfg, "theme", &serde_json::json!({ "x": 1 }));
        assert!(cfg.skills.is_empty(), "non-object values apply nothing");
    }

    /// Regression (moved from merge.rs when the domain loops moved here):
    /// mcp server `env` values must run through `env::resolve_env` —
    /// brace-indirected `{VAR}` values resolve against the process env (empty
    /// when unset), plain values are kept verbatim. Parallel safe: only a
    /// getenv of a never-set var.
    #[test]
    fn apply_domain_resolves_mcp_env_indirection() {
        let mut cfg = Config::default();
        apply_domain(
            &mut cfg,
            "mcp_servers",
            &serde_json::json!({
                "zai-vision": {
                    "enabled": true,
                    "command": "npx",
                    "args": ["-y", "@z_ai/mcp-server@latest"],
                    "env": {
                        "Z_AI_MODE": "ZHIPU",
                        "OPENCODER_TEST_UNSET_KEY": "{OPENCODER_TEST_UNSET_KEY_DOES_NOT_EXIST}"
                    }
                }
            }),
        );
        let srv = cfg
            .mcp_servers
            .get("zai-vision")
            .expect("mcp server applied from domain file object");
        assert!(srv.enabled);
        assert_eq!(srv.command.as_deref(), Some("npx"));
        assert_eq!(srv.args, vec!["-y", "@z_ai/mcp-server@latest"]);
        // literal value (no braces) kept verbatim
        assert_eq!(srv.env.get("Z_AI_MODE").map(String::as_str), Some("ZHIPU"));
        // brace-indirected value routed through resolve_env; unset var -> ""
        assert_eq!(
            srv.env.get("OPENCODER_TEST_UNSET_KEY").map(String::as_str),
            Some("")
        );
    }

    /// Thread-local home isolation for `save_domain` tests: without it the
    /// global write target would be the real `~/.opencoder/<domain>.json`.
    /// With the tempdir as both working dir and home, every fallback layer
    /// resolves inside it (project == global path, no active env marker).
    fn scoped_save_home(dir: &std::path::Path) -> super::super::env::ScopedConfigHome {
        super::super::env::scoped_config_home(dir.to_path_buf())
    }

    // --- Bug #12: whole-domain null patch must not write a literal `null` ---

    #[test]
    fn save_domain_whole_null_empties_existing_file_to_object() {
        let dir = tempfile::tempdir().unwrap();
        let _home = scoped_save_home(dir.path());
        let file = dir.path().join(".opencoder").join("mcp.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"{ "a": { "enabled": true }, "b": { "enabled": false } }"#,
        )
        .unwrap();
        let written = save_domain(dir.path(), "mcp_servers", &serde_json::Value::Null)
            .expect("whole-domain null save must succeed");
        assert_eq!(written, file, "existing project file is the write target");
        let raw = std::fs::read_to_string(&file).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("file must stay parseable, got {raw:?}: {e}"));
        assert_eq!(parsed, serde_json::json!({}), "must be `{{}}`, not `null`");
    }

    #[test]
    fn save_domain_whole_null_on_empty_dir_writes_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        let _home = scoped_save_home(dir.path());
        let written = save_domain(dir.path(), "cli", &serde_json::Value::Null)
            .expect("whole-domain null save into a fresh dir must succeed");
        assert!(written.exists(), "file is created (not deleted) on save");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&written).unwrap())
                .expect("file must be parseable JSON, not a bare `null`");
        assert_eq!(parsed, serde_json::json!({}));
    }

    #[test]
    fn save_domain_after_whole_null_keeps_new_entries_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let _home = scoped_save_home(dir.path());
        let file = dir.path().join(".opencoder").join("mcp.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "stale": { "enabled": true } }"#).unwrap();

        save_domain(dir.path(), "mcp_servers", &serde_json::Value::Null).unwrap();
        let after_null: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after_null, serde_json::json!({}));

        // Pre-fix this started from a literal `null` root, so a follow-up
        // entry-deletion patch whole-replaced instead of merging, and a new
        // entry landed on a non-object root.
        save_domain(
            dir.path(),
            "mcp_servers",
            &serde_json::json!({ "fresh": { "enabled": true } }),
        )
        .unwrap();
        let after_add: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            after_add,
            serde_json::json!({ "fresh": { "enabled": true } }),
            "new entry must land on the normalized `{{}}` root"
        );

        save_domain(
            dir.path(),
            "mcp_servers",
            &serde_json::json!({ "fresh": null }),
        )
        .unwrap();
        let after_del: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            after_del,
            serde_json::json!({}),
            "entry-level null deletion must remove the key, not write a null entry"
        );
    }
}
