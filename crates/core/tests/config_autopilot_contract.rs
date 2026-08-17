//! Contract tests for the three-state autopilot config (`autopilot.mode`):
//! round-trip through save, legacy `enabled` migration, deep-merge
//! preservation, and the default-off guard. Kept in its own file because
//! `config_contract.rs` is already at the 800-line iteration cap.

use std::sync::Mutex;

use opencoder_core::{ApMode, Config};

// Env mutation is process-global; serialize tests that touch the environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct HomeGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg: Option<std::ffi::OsString>,
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match &self.prev_xdg {
            Some(x) => std::env::set_var("XDG_CONFIG_HOME", x),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

fn isolated_home() -> (HomeGuard, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let prev_home = std::env::var_os("HOME");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("HOME", home.path());
    std::env::set_var("XDG_CONFIG_HOME", home.path());
    let cwd = tempfile::tempdir().unwrap();
    (
        HomeGuard {
            prev_home,
            prev_xdg,
        },
        cwd,
    )
}

#[test]
fn mode_roundtrips_all_three_states() {
    let _g = ENV_LOCK.lock().unwrap();
    for (raw, want) in [
        ("off", ApMode::Off),
        ("ap", ApMode::Ap),
        ("review", ApMode::Review),
    ] {
        let (_home_guard, dir) = isolated_home();
        Config::save(
            dir.path(),
            &serde_json::json!({ "autopilot": { "mode": raw } }),
        )
        .unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        assert_eq!(cfg.autopilot.mode, want, "mode {raw:?} round-trips");
    }
}

#[test]
fn legacy_enabled_migrates_instead_of_silently_disabling() {
    let _g = ENV_LOCK.lock().unwrap();
    // enabled=true → ap: a pre-mode user keeps the self-driving loop.
    let (_home_guard, dir) = isolated_home();
    Config::save(
        dir.path(),
        &serde_json::json!({ "autopilot": { "enabled": true } }),
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.autopilot.mode,
        ApMode::Ap,
        "enabled=true migrates to ap"
    );

    // enabled=false → off.
    let (_home_guard, dir) = isolated_home();
    Config::save(
        dir.path(),
        &serde_json::json!({ "autopilot": { "enabled": false } }),
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.autopilot.mode,
        ApMode::Off,
        "enabled=false migrates to off"
    );

    // mode wins when both keys are present (mode is canonical).
    let (_home_guard, dir) = isolated_home();
    Config::save(
        dir.path(),
        &serde_json::json!({ "autopilot": { "enabled": true, "mode": "review" } }),
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.autopilot.mode,
        ApMode::Review,
        "mode beats legacy enabled"
    );
}

#[test]
fn mode_survives_partial_deep_merge() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    Config::save(
        dir.path(),
        &serde_json::json!({ "autopilot": { "mode": "review", "max_iterations": 5 } }),
    )
    .unwrap();
    // Patch a sibling sub-key only — the object must deep-merge, not replace.
    Config::save(
        dir.path(),
        &serde_json::json!({ "autopilot": { "max_iterations": 20 } }),
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.autopilot.mode,
        ApMode::Review,
        "mode preserved by deep merge"
    );
    assert_eq!(cfg.autopilot.max_iterations, 20, "max_iterations patched");
}

#[test]
fn default_mode_is_off() {
    let _g = ENV_LOCK.lock().unwrap();
    let (_home_guard, dir) = isolated_home();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(
        cfg.autopilot.mode,
        ApMode::Off,
        "fresh install defaults to off"
    );
    assert_eq!(Config::default().autopilot.mode, ApMode::Off);
}
