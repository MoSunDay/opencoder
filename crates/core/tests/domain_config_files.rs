//! Integration tests for the domain config files (`mcp.json` / `cli.json` /
//! `skills.json`): the three map domains are hard-cut from `config.json` and
//! live only in dedicated files. Covers the pinned decisions — two-candidate
//! lookup (project shadows global), write-target resolution (双无写全局),
//! null-deletes-key, corrupt-refuses-write (损坏拒写), the legacy
//! config.json hard-cut, and `Config::save` split-routing (分流).
//!
//! Every test isolates the global home via the thread-local
//! `scoped_config_home` override + `tempfile::tempdir()`; the process env is
//! never mutated.

use std::path::{Path, PathBuf};

use opencoder_core::{Config, scoped_config_home};

/// `<home>/.opencoder/<name>` — the global domain-file location.
fn global_domain_file(home: &Path, name: &str) -> PathBuf {
    home.join(".opencoder").join(name)
}

/// `<work>/.opencoder/<name>` — the project domain-file location.
fn project_domain_file(work: &Path, name: &str) -> PathBuf {
    work.join(".opencoder").join(name)
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write_json(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

// --- Decision 2: neither file exists -> the GLOBAL domain file is created ---
#[test]
fn save_creates_global_domain_file_when_neither_exists() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    let written = Config::save(
        work.path(),
        &serde_json::json!({ "skills": { "review": { "enabled": true } } }),
    )
    .unwrap();

    let global = global_domain_file(home.path(), "skills.json");
    assert_eq!(written, global, "neither candidate exists -> create global");
    assert_eq!(
        read_json(&global)["review"]["enabled"],
        serde_json::json!(true)
    );
    assert!(
        !project_domain_file(work.path(), "skills.json").exists(),
        "no project domain file should be created"
    );

    // the freshly-created global file is exactly what load picks up
    let cfg = Config::load(work.path()).unwrap();
    assert_eq!(cfg.enabled_skill_names(), vec!["review".to_string()]);
}

// --- Decision 1: both exist -> ONLY the project file applies (shadowing) ---
#[test]
fn project_domain_file_shadows_global_entirely() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    write_json(
        &global_domain_file(home.path(), "skills.json"),
        r#"{"alpha":{"enabled":true},"shared":{"enabled":true}}"#,
    );
    write_json(
        &project_domain_file(work.path(), "skills.json"),
        r#"{"beta":{"enabled":true}}"#,
    );

    let cfg = Config::load(work.path()).unwrap();
    // the project file wins ENTIRELY — no per-key merge across the two files
    assert_eq!(cfg.skills.len(), 1, "only project entries load");
    assert!(cfg.skills.contains_key("beta"));
    assert!(!cfg.skills.contains_key("alpha"));
    assert!(!cfg.skills.contains_key("shared"));
    assert_eq!(cfg.enabled_skill_names(), vec!["beta".to_string()]);
}

// --- Decision 5: a null patch entry deletes the key from the domain file ---
#[test]
fn null_patch_entry_deletes_domain_key() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    let global = global_domain_file(home.path(), "skills.json");
    write_json(
        &global,
        r#"{"review":{"enabled":true},"other":{"enabled":true}}"#,
    );

    let written = Config::save(
        work.path(),
        &serde_json::json!({ "skills": { "review": null } }),
    )
    .unwrap();
    assert_eq!(written, global, "global file exists -> it is the target");

    let on_disk = read_json(&global);
    assert!(
        !on_disk.as_object().unwrap().contains_key("review"),
        "null entry must delete the key from the domain file"
    );
    assert_eq!(on_disk["other"]["enabled"], serde_json::json!(true));

    let cfg = Config::load(work.path()).unwrap();
    assert!(!cfg.skills.contains_key("review"), "deleted key is gone");
    assert_eq!(cfg.enabled_skill_names(), vec!["other".to_string()]);
}

// --- Decision 4: corrupt existing target refuses the write (损坏拒写) ---
#[test]
fn corrupt_domain_file_refuses_write_and_loads_as_absent() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    let target = global_domain_file(home.path(), "skills.json");
    let corrupt = "{ this is :: not valid json";
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, corrupt).unwrap();

    let res = Config::save(
        work.path(),
        &serde_json::json!({ "skills": { "x": { "enabled": true } } }),
    );
    assert!(res.is_err(), "corrupt target must refuse the write");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        corrupt,
        "corrupt file must be left byte-for-byte untouched"
    );

    // Decision 3 (read side): corrupt is warned + treated as absent, not an error
    let cfg = Config::load(work.path()).unwrap();
    assert!(cfg.skills.is_empty(), "corrupt domain file loads as absent");
}

// --- Decision 3: a valid-but-non-object domain file is treated as absent ---
#[test]
fn non_object_domain_file_loads_as_absent() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    write_json(&global_domain_file(home.path(), "cli.json"), "[1, 2, 3]");
    let cfg = Config::load(work.path()).unwrap();
    assert!(cfg.cli.is_empty(), "non-object domain file loads as absent");
    assert!(cfg.enabled_cli().is_empty());
}

// --- Decision 6: legacy config.json domain keys are ignored (hard cut) ---
#[test]
fn legacy_config_json_domain_keys_are_ignored_on_load() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    write_json(
        &work.path().join("opencoder.json"),
        r#"{
            "theme": "light",
            "mcp_servers": { "srv": { "enabled": true, "command": "npx" } },
            "cli": { "git": { "enabled": true, "content": "use git" } },
            "skills": { "review": { "enabled": true } }
        }"#,
    );

    let cfg = Config::load(work.path()).unwrap();
    assert!(cfg.mcp_servers.is_empty(), "legacy `mcp_servers` ignored");
    assert!(cfg.cli.is_empty(), "legacy `cli` ignored");
    assert!(cfg.skills.is_empty(), "legacy `skills` ignored");
    assert!(cfg.enabled_skill_names().is_empty());
    assert!(cfg.enabled_mcp_servers().is_empty());
    assert_eq!(
        cfg.theme, "light",
        "non-domain keys still load from config.json"
    );
}

// --- Decision 7: mixed patch splits — config.json gets the remainder ---
#[test]
fn mixed_patch_routes_domain_and_config_keys_separately() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    let written = Config::save(
        work.path(),
        &serde_json::json!({
            "theme": "light",
            "skills": { "review": { "enabled": true } }
        }),
    )
    .unwrap();

    let config_json = work.path().join("opencoder.json");
    assert_eq!(
        written, config_json,
        "non-empty config remainder -> config.json path (domain writes still happen)"
    );
    let config_disk = read_json(&config_json);
    assert_eq!(config_disk["theme"], serde_json::json!("light"));
    assert!(
        !config_disk.as_object().unwrap().contains_key("skills"),
        "skills must NOT land in config.json"
    );
    let skills_disk = read_json(&global_domain_file(home.path(), "skills.json"));
    assert_eq!(skills_disk["review"]["enabled"], serde_json::json!(true));

    // roundtrip: both halves come back from their own files
    let cfg = Config::load(work.path()).unwrap();
    assert_eq!(cfg.theme, "light");
    assert_eq!(cfg.enabled_skill_names(), vec!["review".to_string()]);
}

// --- Decision 7 (tail): domain-only patch returns the domain target ---
#[test]
fn domain_only_patch_returns_domain_target_and_skips_config_json() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    let written = Config::save(
        work.path(),
        &serde_json::json!({ "cli": { "git": { "enabled": true, "content": "use git" } } }),
    )
    .unwrap();

    let global_cli = global_domain_file(home.path(), "cli.json");
    assert_eq!(
        written, global_cli,
        "domain-only patch -> last domain target"
    );
    assert!(
        !work.path().join("opencoder.json").exists(),
        "no config.json should be created for a domain-only patch"
    );
    assert!(
        !project_domain_file(work.path(), "cli.json").exists(),
        "no project domain file should be created (global is the default target)"
    );

    let cfg = Config::load(work.path()).unwrap();
    let cli = cfg.enabled_cli();
    assert_eq!(cli.len(), 1);
    assert_eq!(cli[0].0, "git");
    assert_eq!(cli[0].1.content, "use git");
}

// --- Decision 7 (tail): empty patch keeps the legacy config.json behavior ---
#[test]
fn empty_patch_still_writes_config_json() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    let written = Config::save(work.path(), &serde_json::json!({})).unwrap();
    assert_eq!(written, work.path().join("opencoder.json"));
    assert!(work.path().join("opencoder.json").exists());
}

// --- load picks up entries from every domain file ---
#[test]
fn load_reads_entries_from_all_three_domain_files() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _guard = scoped_config_home(home.path().to_path_buf());

    write_json(
        &global_domain_file(home.path(), "mcp.json"),
        r#"{"srv":{"enabled":true,"command":"npx","args":["-y","pkg"]}}"#,
    );
    write_json(
        &global_domain_file(home.path(), "cli.json"),
        r#"{"git":{"enabled":true,"content":"c"}}"#,
    );
    write_json(
        &global_domain_file(home.path(), "skills.json"),
        r#"{"review":{"enabled":true}}"#,
    );

    let cfg = Config::load(work.path()).unwrap();
    let servers = cfg.enabled_mcp_servers();
    assert_eq!(servers.len(), 1, "enabled_mcp_servers reflects mcp.json");
    assert_eq!(servers[0].0, "srv");
    assert_eq!(servers[0].1.command.as_deref(), Some("npx"));
    assert_eq!(servers[0].1.args, vec!["-y".to_string(), "pkg".to_string()]);

    let cli = cfg.enabled_cli();
    assert_eq!(cli.len(), 1, "enabled_cli reflects cli.json");
    assert_eq!(cli[0].0, "git");

    assert_eq!(
        cfg.enabled_skill_names(),
        vec!["review".to_string()],
        "enabled_skill_names reflects skills.json"
    );
}
