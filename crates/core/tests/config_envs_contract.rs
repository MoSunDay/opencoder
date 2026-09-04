//! `/envs` contract: named env config sets layered into resolution as
//! **project > env > ~/.opencoder > XDG**. Every test isolates the global
//! home via the thread-local `scoped_config_home` override + tempdirs; the
//! process env is never mutated.

use std::path::{Path, PathBuf};

use opencoder_core::{
    active_env, create_env, delete_env, list_envs, recapture_env, scoped_config_home,
    set_active_env, set_active_env_checked, ApMode, Config,
};

fn write_json(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// `<home>/.opencoder/envs/<name>/<file>`
fn env_file(home: &Path, name: &str, file: &str) -> PathBuf {
    home.join(".opencoder").join("envs").join(name).join(file)
}

/// Build an env dir with the given config.json body (+ optional domain files).
fn make_env(home: &Path, name: &str, config_body: &str) {
    write_json(&env_file(home, name, "config.json"), config_body);
}

/// Precondition helper: two-layer world — global `config.json` provides the
/// base (provider + model G), env `work` overrides the model.
fn base_world(home: &Path, work: &Path) {
    write_json(
        &home.join(".opencoder").join("config.json"),
        r#"{"provider":{"base_url":"https://g.example","api_key":"gk"},"model":"global/m"}"#,
    );
    make_env(home, "work", r#"{"model":"env/m"}"#);
    let _ = work;
}

#[test]
fn env_layer_sits_between_project_and_global() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());

    // no env active: base behavior
    assert_eq!(Config::load(work.path()).unwrap().model, "global/m");

    // env active: env shadows global, keys not in env still come from global
    set_active_env(Some("work")).unwrap();
    let cfg = Config::load(work.path()).unwrap();
    assert_eq!(cfg.model, "env/m");
    assert_eq!(cfg.provider.base_url, "https://g.example");
    assert_eq!(cfg.provider.api_key.as_deref(), Some("gk"));

    // project shadows env
    write_json(
        &work.path().join("opencoder.json"),
        r#"{"model":"project/m"}"#,
    );
    assert_eq!(Config::load(work.path()).unwrap().model, "project/m");

    // deactivation restores the base chain verbatim
    set_active_env(None).unwrap();
    assert_eq!(Config::load(work.path()).unwrap().model, "project/m");
}

#[test]
fn domain_files_shadow_project_env_global() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());

    write_json(
        &home.path().join(".opencoder").join("mcp.json"),
        r#"{"g1":{"enabled":true,"command":"g"},"g2":{"enabled":true,"command":"g"}}"#,
    );
    make_env(home.path(), "work", "{}");
    write_json(
        &env_file(home.path(), "work", "mcp.json"),
        r#"{"e1":{"enabled":true,"command":"e"}}"#,
    );
    set_active_env(Some("work")).unwrap();

    // env shadows global entirely (no per-key merge)
    let cfg = Config::load(work.path()).unwrap();
    let names: Vec<String> = cfg.mcp_servers.keys().cloned().collect();
    assert_eq!(names, vec!["e1".to_string()]);

    // project shadows env entirely
    write_json(
        &work.path().join(".opencoder").join("mcp.json"),
        r#"{"p1":{"enabled":true,"command":"p"}}"#,
    );
    let cfg = Config::load(work.path()).unwrap();
    assert_eq!(
        cfg.mcp_servers.keys().cloned().collect::<Vec<_>>(),
        vec!["p1".to_string()]
    );

    // write target while active (+ no project file): new domain saves land in
    // the env dir, base global file stays untouched
    std::fs::remove_file(work.path().join(".opencoder").join("mcp.json")).unwrap();
    let before = std::fs::read_to_string(home.path().join(".opencoder").join("mcp.json")).unwrap();
    let written = Config::save(
        work.path(),
        &serde_json::json!({"mcp_servers": {"e2": {"enabled": true, "command": "e2"}}}),
    )
    .unwrap();
    assert_eq!(written, env_file(home.path(), "work", "mcp.json"));
    assert_eq!(
        std::fs::read_to_string(home.path().join(".opencoder").join("mcp.json")).unwrap(),
        before,
        "base global mcp.json untouched while env active"
    );
}

#[test]
fn stale_marker_falls_back_to_base() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    set_active_env(Some("work")).unwrap();

    // env dir removed behind the marker's back
    std::fs::remove_dir_all(home.path().join(".opencoder").join("envs").join("work")).unwrap();
    assert!(active_env().is_none(), "stale marker deactivates");
    assert_eq!(Config::load(work.path()).unwrap().model, "global/m");
    assert!(list_envs().is_empty());

    // save_target no longer env-routed: with an editable global file present
    // it targets the global config (pre-env behavior)
    assert_eq!(
        Config::save_target(work.path()),
        home.path().join(".opencoder").join("config.json")
    );
}

#[test]
fn capture_snapshots_base_chain_without_env_overlay() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());

    // global base with model+provider AND stray domain keys in config.json
    write_json(
        &home.path().join(".opencoder").join("config.json"),
        r#"{"model":"global/m","provider":{"api_key":"gk"},"mcp_servers":{"x":{"enabled":true}},"skills":{"s":{"enabled":true}},"autopilot":{"mode":"ap"}}"#,
    );
    write_json(
        &home.path().join(".opencoder").join("ap.json"),
        r#"{"mode":"review"}"#,
    );
    write_json(
        &home.path().join(".opencoder").join("cli.json"),
        r#"{"git":{"enabled":true,"content":"use git"}}"#,
    );
    // project layer is part of the capture (WYSIWYG)
    write_json(&work.path().join("opencoder.json"), r#"{"fps":24}"#);
    // another env is ACTIVE during capture: it must NOT leak into the capture
    make_env(home.path(), "other", r#"{"model":"other/m"}"#);
    set_active_env(Some("other")).unwrap();

    create_env("shot", work.path(), true).unwrap();
    let captured = read_json(&env_file(home.path(), "shot", "config.json"));
    assert_eq!(captured["model"], "global/m", "active env excluded");
    assert_eq!(captured["fps"], 24, "project layer included");
    assert_eq!(captured["provider"]["api_key"], "gk");
    for k in ["mcp_servers", "cli", "skills", "autopilot"] {
        assert!(captured.get(k).is_none(), "domain key {k} stripped");
    }
    // domain files snapshotted from the base chain
    assert_eq!(
        read_json(&env_file(home.path(), "shot", "cli.json"))["git"]["content"],
        "use git"
    );
    assert!(
        !env_file(home.path(), "shot", "mcp.json").exists()
            && !env_file(home.path(), "shot", "skills.json").exists(),
        "no base source -> no env domain file"
    );
    assert_eq!(
        read_json(&env_file(home.path(), "shot", "ap.json"))["mode"],
        serde_json::json!("review"),
        "ap.json snapshotted from the base chain"
    );

    // activating the captured env reproduces the base file view exactly
    set_active_env(Some("shot")).unwrap();
    let cfg = Config::load(work.path()).unwrap();
    assert_eq!(cfg.model, "global/m");
    assert_eq!(cfg.fps, Some(24));
    assert_eq!(cfg.cli["git"].content, "use git");
    assert_eq!(cfg.autopilot.mode, opencoder_core::ApMode::Review);
}

#[test]
fn recapture_replaces_stale_env_files() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path()); // env "work" already exists
    write_json(
        &env_file(home.path(), "work", "mcp.json"),
        r#"{"old":{"enabled":true}}"#,
    );

    // base chain changes; recapture must replace config.json AND drop the
    // stale env mcp.json (its source is gone)
    write_json(
        &home.path().join(".opencoder").join("config.json"),
        r#"{"provider":{"base_url":"https://g2.example","api_key":"gk2"},"model":"global2/m"}"#,
    );
    recapture_env("work", work.path()).unwrap();
    let cfg_json = read_json(&env_file(home.path(), "work", "config.json"));
    assert_eq!(cfg_json["model"], "global2/m");
    assert!(!env_file(home.path(), "work", "mcp.json").exists());
    assert!(recapture_env("missing", work.path()).is_err());
}

#[test]
fn save_routes_to_env_while_active_and_back_after_deactivation() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    let global = home.path().join(".opencoder").join("config.json");
    let before = std::fs::read_to_string(&global).unwrap();

    set_active_env(Some("work")).unwrap();
    let written = Config::save(work.path(), &serde_json::json!({"model": "env/mo"})).unwrap();
    assert_eq!(written, env_file(home.path(), "work", "config.json"));
    assert_eq!(std::fs::read_to_string(&global).unwrap(), before);
    assert_eq!(Config::load(work.path()).unwrap().model, "env/mo");

    // deactivate: the very same save now targets the base global file
    set_active_env(None).unwrap();
    let written = Config::save(work.path(), &serde_json::json!({"model": "global/mo3"})).unwrap();
    assert_eq!(written, global);
    assert_eq!(Config::load(work.path()).unwrap().model, "global/mo3");
}

#[test]
fn save_creates_env_config_when_nothing_editable_exists() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    make_env(home.path(), "fresh", "{}");
    set_active_env(Some("fresh")).unwrap();
    // neither project files nor the (empty) env config hold editable keys
    let target = Config::save_target(work.path());
    assert_eq!(target, env_file(home.path(), "fresh", "config.json"));
    let written = Config::save(work.path(), &serde_json::json!({"model": "prov/mo"})).unwrap();
    assert_eq!(written, target);
}

#[test]
fn project_files_still_win_save_target_while_env_active() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    write_json(
        &work.path().join("opencoder.json"),
        r#"{"model":"project/m"}"#,
    );
    set_active_env(Some("work")).unwrap();
    // project file holds editable keys -> it stays the save target
    assert_eq!(
        Config::save_target(work.path()),
        work.path().join("opencoder.json")
    );
}

#[test]
fn delete_active_env_clears_marker_and_restores_base() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    set_active_env(Some("work")).unwrap();
    assert_eq!(Config::load(work.path()).unwrap().model, "env/m");

    delete_env("work").unwrap();
    assert!(active_env().is_none());
    assert_eq!(Config::load(work.path()).unwrap().model, "global/m");
    assert!(!env_file(home.path(), "work", "config.json").exists());
}

#[test]
fn env_files_are_owner_only_on_unix() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    create_env("shot", work.path(), true).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(env_file(home.path(), "shot", "config.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "captured config.json is owner-only");
    }
}

/// Core acceptance for the autopilot domain: `ap.json` is env-scoped, so the
/// ap mode FOLLOWS the active env. The regression this pins: `/ap` saves used
/// to be routed by `save_target` into the project config.json (any editable
/// key there qualified), pinning the mode in the project layer where it
/// shadowed every env — switching envs never changed it.
#[test]
fn autopilot_mode_follows_env_activation_switch_and_deactivation() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());

    // project opencoder.json holds an editable key (model) — it must NOT
    // capture autopilot anymore
    write_json(&work.path().join("opencoder.json"), r#"{"model":"proj/m"}"#);
    // base global ap.json: off
    write_json(
        &home.path().join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    );
    // two envs with different ap modes
    make_env(home.path(), "alpha", r#"{"fps":10}"#);
    write_json(
        &env_file(home.path(), "alpha", "ap.json"),
        r#"{"mode":"ap"}"#,
    );
    make_env(home.path(), "beta", r#"{"fps":10}"#);
    write_json(
        &env_file(home.path(), "beta", "ap.json"),
        r#"{"mode":"review"}"#,
    );

    // base: global ap.json wins
    assert_eq!(
        Config::load(work.path()).unwrap().autopilot.mode,
        ApMode::Off
    );

    // env A active: A's mode; a `/ap`-style save lands in envs/alpha/ap.json
    set_active_env(Some("alpha")).unwrap();
    assert_eq!(
        Config::load(work.path()).unwrap().autopilot.mode,
        ApMode::Ap
    );
    let written = Config::save(
        work.path(),
        &serde_json::json!({ "autopilot": { "mode": "review" } }),
    )
    .unwrap();
    assert_eq!(written, env_file(home.path(), "alpha", "ap.json"));
    assert_eq!(
        read_json(&env_file(home.path(), "alpha", "ap.json"))["mode"],
        serde_json::json!("review"),
        "edit while env active lands in the env layer"
    );
    assert_eq!(
        read_json(&home.path().join(".opencoder").join("ap.json"))["mode"],
        serde_json::json!("off"),
        "base global ap.json untouched while env active"
    );
    assert_eq!(
        read_json(&work.path().join("opencoder.json")),
        serde_json::json!({ "model": "proj/m" }),
        "project config.json must not gain an autopilot key"
    );

    // switch to env B: B's mode applies (previously the project layer pinned it)
    set_active_env(Some("beta")).unwrap();
    assert_eq!(
        Config::load(work.path()).unwrap().autopilot.mode,
        ApMode::Review,
        "switching envs must switch the ap mode"
    );

    // deactivate: back to the base chain
    set_active_env(None).unwrap();
    assert_eq!(
        Config::load(work.path()).unwrap().autopilot.mode,
        ApMode::Off,
        "deactivation restores the base ap.json"
    );
}

/// E-1: config/domain saves that land in the active env dir must be
/// owner-only (0o600) — these files embed provider api keys. A pre-existing
/// env file written before the contract (plain 0644) is chmod-converged by
/// the next save.
#[cfg(unix)]
#[test]
fn env_layer_saves_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    set_active_env(Some("work")).unwrap();

    // config.json save: new file in the env dir -> 0600
    let written = Config::save(work.path(), &serde_json::json!({"model": "env/mo"})).unwrap();
    let mode = std::fs::metadata(&written).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        format!("{mode:o}"),
        "600",
        "env config.json save must be 0600"
    );

    // mcp.json domain save: same contract
    let written = Config::save(
        work.path(),
        &serde_json::json!({"mcp_servers": {"e1": {"enabled": true, "command": "e1"}}}),
    )
    .unwrap();
    let mode = std::fs::metadata(&written).unwrap().permissions().mode() & 0o7777;
    assert_eq!(format!("{mode:o}"), "600", "env domain save must be 0600");

    // Convergence: a 0644 file that predates the contract is repaired on the
    // next save into it.
    let env_config = env_file(home.path(), "work", "config.json");
    std::fs::set_permissions(&env_config, std::fs::Permissions::from_mode(0o644)).unwrap();
    Config::save(work.path(), &serde_json::json!({"fps": 12})).unwrap();
    let mode = std::fs::metadata(&env_config).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        format!("{mode:o}"),
        "600",
        "pre-existing env file must converge to 0600"
    );

    // Non-env targets keep the default behavior (no forced 0600): save into
    // a project dir with no existing candidates creates opencoder.json.
    set_active_env(None).unwrap();
    drop(_iso); // release the scoped global home before re-scoping
    let fresh_home = tempfile::tempdir().unwrap();
    let _iso2 = scoped_config_home(fresh_home.path().to_path_buf());
    let fresh = tempfile::tempdir().unwrap();
    let project = fresh.path().join("opencoder.json");
    Config::save(fresh.path(), &serde_json::json!({"fps": 30})).unwrap();
    let mode = std::fs::metadata(&project).unwrap().permissions().mode() & 0o7777;
    assert_ne!(
        format!("{mode:o}"),
        "600",
        "project saves are not forced to 0600"
    );
}

/// E-2: activation preflight — a corrupt env config.json must be rejected by
/// `set_active_env_checked` (instead of poisoning the next process start),
/// with the previous marker state restored. A resolvable env activates
/// normally, and deactivation passes through.
#[test]
fn activation_preflight_rejects_unresolvable_env_and_restores_marker() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());

    // Good env: activates and stays active.
    make_env(home.path(), "good", r#"{"model":"good/m"}"#);
    set_active_env_checked(Some("good"), work.path()).unwrap();
    assert_eq!(active_env().as_deref(), Some("good"));
    assert_eq!(Config::load(work.path()).unwrap().model, "good/m");

    // Corrupt env: rejected, marker restored to "good".
    write_json(&env_file(home.path(), "bad", "config.json"), "{ not json");
    let error = set_active_env_checked(Some("bad"), work.path()).unwrap_err();
    assert!(
        error.to_string().contains("unresolvable"),
        "error must name the resolution failure: {error}"
    );
    assert_eq!(
        active_env().as_deref(),
        Some("good"),
        "failed preflight must restore the previous activation"
    );
    assert_eq!(Config::load(work.path()).unwrap().model, "good/m");

    // From no-activation: failed preflight clears the marker again.
    set_active_env_checked(None, work.path()).unwrap();
    let error = set_active_env_checked(Some("bad"), work.path()).unwrap_err();
    assert!(error.to_string().contains("unresolvable"));
    assert_eq!(active_env(), None, "no env may be left active");
}

/// E-3: the active marker is replaced atomically (temp + rename) — the
/// marker file must never carry a temp sibling's name, and rapid rewrites
/// always leave exactly one parseable name.
#[test]
fn rapid_marker_rewrites_stay_parseable() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = scoped_config_home(home.path().to_path_buf());
    base_world(home.path(), work.path());
    make_env(home.path(), "x", "{}");
    make_env(home.path(), "y", "{}");

    for name in ["x", "y", "x", "y", "y"] {
        set_active_env(Some(name)).unwrap();
        assert_eq!(active_env().as_deref(), Some(name));
    }
    // No temp siblings left behind.
    let entries: Vec<String> = std::fs::read_dir(home.path().join(".opencoder").join("envs"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        entries
            .iter()
            .all(|n| n == "x" || n == "y" || n == "work" || n == "active"),
        "no temp marker siblings may survive: {entries:?}"
    );
}
