//! Unit tests for [`super`] (agent meta + shared resource read path),
//! split out of `meta.rs` to respect the file line budget. Mirrors the
//! envs.rs in-file test conventions.

use std::sync::{Mutex, MutexGuard};

use super::*;

/// Serializes tests that touch the process-global agents-root override.
/// `pub(crate)` so the resolution tests in `agent::tests` share the same
/// lock — the override is process-global, so without it parallel tests
/// race.
pub(crate) static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

/// Point the agents root at a fresh tempdir under the override lock.
/// Tests reading `agents_dir()` must hold the lock for the whole body:
/// the override is process-global, so without it parallel tests race.
fn scoped() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

/// One agent reference card: `<name>/meta.json` (no version dirs — the
/// card references shared pools by name).
fn make_agent(root: &std::path::Path, name: &str) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    std::fs::write(root.join(name).join("meta.json"), "{}").unwrap();
}

/// `<cat>/<name>/meta.json` pointing `current` at `v`, plus optional
/// `v{v}/` files (`(filename, body)` pairs).
fn make_resource(root: &std::path::Path, cat: &str, name: &str, v: u32, files: &[(&str, &str)]) {
    let res = root.join(cat).join(name);
    let vdir = res.join(format!("v{v}"));
    std::fs::create_dir_all(&vdir).unwrap();
    for (file, body) in files {
        let path = vdir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    std::fs::write(
        res.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": {v}, "history": [{v}] }}"#),
    )
    .unwrap();
}

/// Agent names: charset/length rules hold; the marker name AND the four
/// pool dir names are reserved in the agent namespace.
#[test]
fn validate_agent_name_accepts_and_rejects() {
    assert!(validate_agent_name("work").is_ok());
    assert!(validate_agent_name("MyAgent-2.b").is_ok());
    for bad in [
        "",
        ".",
        "..",
        "a/b",
        "../x",
        "a b",
        "中文",
        "active",
        "prompts",
        "skills",
        "tools",
        "memory",
        &"x".repeat(49),
    ] {
        assert!(
            validate_agent_name(bad).is_err(),
            "{bad:?} should be invalid"
        );
    }
}

/// Resource names: same charset/length rules, but the reserved set is
/// just non-empty/`.`/`..`/charset/≤48 — they live under their category
/// dir, so `active` (and even a category token) is a legal resource
/// name. Unknown categories are rejected.
#[test]
fn validate_resource_name_rules() {
    for cat in AGENT_CATEGORIES {
        assert!(validate_resource_name(cat, "default").is_ok(), "{cat}");
        assert!(validate_resource_name(cat, "active").is_ok(), "{cat}");
        assert!(validate_resource_name(cat, "My-Pack_2.x").is_ok(), "{cat}");
    }
    for bad in ["", ".", "..", "a/b", "a b", "中文", &"x".repeat(49)] {
        assert!(
            validate_resource_name("prompts", bad).is_err(),
            "{bad:?} should be invalid"
        );
    }
    assert!(validate_resource_name("nosuch", "ok").is_err());
    assert!(validate_resource_name("", "ok").is_err());
    assert_eq!(category_dir("nosuch"), None);
}

/// Marker roundtrip: set → read back; clear → `None`; clearing an
/// already-clear root is not an error; unknown agents are rejected
/// *before* any marker write.
#[test]
fn marker_roundtrip_and_unknown_agent_rejected() {
    let (dir, _g) = scoped();
    let root = dir.path();
    make_agent(root, "alpha");
    set_active_agent(Some("alpha")).unwrap();
    assert_eq!(active_agent().as_deref(), Some("alpha"));
    // Unknown agent → InvalidInput, marker untouched.
    let err = set_active_agent(Some("ghost")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(active_agent().as_deref(), Some("alpha"));
    // Reserved pool dir names are not agents either.
    assert!(set_active_agent(Some("prompts")).is_err());
    // Clear + idempotent clear.
    set_active_agent(None).unwrap();
    assert_eq!(active_agent(), None);
    set_active_agent(None).unwrap();
}

/// `meta.json` parsing is backward-tolerant: unknown keys and partial
/// reference cards parse; corrupt JSON degrades to `None`.
#[test]
fn read_agent_meta_tolerant_and_corrupt() {
    let (dir, _g) = scoped();
    let root = dir.path();
    make_agent(root, "good");
    assert_eq!(read_agent_meta("good"), Some(AgentMeta::default()));
    std::fs::write(root.join("good").join("meta.json"), "{ not json").unwrap();
    assert_eq!(read_agent_meta("good"), None);
    std::fs::write(
        root.join("good").join("meta.json"),
        r#"{ "name": "good", "current": { "prompt": "default", "memory": "longterm" },
             "history": [ { "at": "t", "field": "prompt", "from": null, "to": "default" } ],
             "references": { "memory": true }, "unknown_key": 1 }"#,
    )
    .unwrap();
    let meta = read_agent_meta("good").expect("unknown keys must be tolerated");
    assert_eq!(meta.name, "good");
    assert_eq!(meta.current.prompt.as_deref(), Some("default"));
    assert_eq!(meta.current.memory.as_deref(), Some("longterm"));
    assert_eq!(meta.current.tools, None, "absent category = no ref");
    assert_eq!(meta.history.len(), 1);
    assert_eq!(meta.history[0].field, "prompt");
    assert_eq!(meta.history[0].to.as_deref(), Some("default"));
    assert!(meta.references.memory);
    assert!(meta.references.prompt_files.is_empty());
    assert_eq!(read_agent_meta("ghost"), None);
}

/// Listing is sorted and skips the reserved non-agent names (marker plus
/// the four pool dirs); an invalid name never resolves to a path.
#[test]
fn list_agents_sorted_and_skips_reserved() {
    let (dir, _g) = scoped();
    let root = dir.path();
    make_agent(root, "beta");
    make_agent(root, "alpha");
    std::fs::create_dir_all(root.join(ACTIVE_MARKER)).unwrap(); // legacy leftover
    for cat in AGENT_CATEGORIES {
        std::fs::create_dir_all(root.join(cat)).unwrap();
    }
    assert_eq!(list_agents(), vec!["alpha".to_string(), "beta".to_string()]);
    assert_eq!(agent_dir("a/b"), None);
    assert_eq!(agent_dir(""), None);
    assert_eq!(agent_dir("prompts"), None);
}

/// Preflight rollback: a failing check restores the previous marker and
/// returns `InvalidData`; a passing check keeps the new marker;
/// deactivation passes the closure through untouched.
#[test]
fn preflight_failure_rolls_back_marker() {
    let (dir, _g) = scoped();
    let root = dir.path();
    make_agent(root, "alpha");
    make_agent(root, "beta");
    set_active_agent(Some("alpha")).unwrap();
    let err = set_active_agent_checked(Some("beta"), || Err("dry-run failed".into())).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        active_agent().as_deref(),
        Some("alpha"),
        "marker must roll back"
    );
    set_active_agent_checked(Some("beta"), || Ok(())).unwrap();
    assert_eq!(active_agent().as_deref(), Some("beta"));
    set_active_agent_checked(None, || panic!("deactivation must not run the check")).unwrap();
    assert_eq!(active_agent(), None);
}

/// Resource meta read + version-dir resolution: `current` follows the
/// pool's meta (the bump propagates), `current: 0` means absent, and the
/// current dir must exist as a directory.
#[test]
fn resource_meta_and_version_dirs_follow_current() {
    let (dir, _g) = scoped();
    let root = dir.path();
    make_resource(root, "prompts", "default", 1, &[("soul.md", "s1")]);
    let meta = read_resource_meta("prompts", "default").unwrap();
    assert_eq!(
        (meta.name.as_str(), meta.current, meta.history.as_slice()),
        ("default", 1, &[1][..])
    );
    let v1 = root.join("prompts").join("default").join("v1");
    assert_eq!(
        resource_current_version_dir("prompts", "default"),
        Some(v1.clone())
    );
    // Pure path computation: no existence check, any version number.
    assert_eq!(
        resource_version_dir("prompts", "default", 7),
        Some(root.join("prompts").join("default").join("v7"))
    );
    // Bump by hand: v2 dir + meta current=2 → both readers move to v2.
    let v2 = root.join("prompts").join("default").join("v2");
    std::fs::create_dir_all(&v2).unwrap();
    std::fs::write(v2.join("soul.md"), "s2").unwrap();
    std::fs::write(
        root.join("prompts").join("default").join("meta.json"),
        r#"{ "name": "default", "current": 2, "history": [1, 2] }"#,
    )
    .unwrap();
    assert_eq!(resource_current_version_dir("prompts", "default"), Some(v2));
    // current: 0 ⇒ absent.
    make_resource(root, "memory", "blank", 0, &[]);
    assert_eq!(read_resource_meta("memory", "blank").unwrap().current, 0);
    assert_eq!(resource_current_version_dir("memory", "blank"), None);
    // Missing resource / corrupt meta ⇒ silent None.
    assert_eq!(read_resource_meta("prompts", "ghost"), None);
    assert_eq!(resource_current_version_dir("prompts", "ghost"), None);
    make_resource(root, "skills", "corrupt", 1, &[]);
    std::fs::write(root.join("skills/corrupt/meta.json"), "{ not json").unwrap();
    assert_eq!(read_resource_meta("skills", "corrupt"), None);
    assert_eq!(resource_current_version_dir("skills", "corrupt"), None);
    // Unknown category ⇒ silent None everywhere.
    assert_eq!(read_resource_meta("nosuch", "x"), None);
    assert_eq!(resource_version_dir("nosuch", "x", 1), None);
}

/// `list_resources`: sorted, skips invalid names, silent empty for an
/// unknown category or an absent pool dir.
#[test]
fn list_resources_sorted_silent_and_filtered() {
    let (dir, _g) = scoped();
    let root = dir.path();
    make_resource(root, "tools", "zeta", 1, &[("run.sh", "#!/bin/sh\n")]);
    make_resource(root, "tools", "alpha", 1, &[]);
    std::fs::create_dir_all(root.join("tools").join("a b")).unwrap(); // stray dir
    std::fs::write(root.join("tools").join("plain.txt"), "x").unwrap(); // plain file
    assert_eq!(
        list_resources("tools"),
        vec!["alpha".to_string(), "zeta".to_string()]
    );
    assert!(list_resources("prompts").is_empty());
    assert!(list_resources("nosuch").is_empty());
}

/// Reference helpers shape: `agent_skill_roots`/`agent_tools_dirs` return
/// 0 or 1 entries; the active-agent wrappers follow the marker; and
/// `all_tools_dirs` unions every tools pool's current version dir,
/// sorted, skipping version-less resources.
#[test]
fn agent_ref_helpers_and_all_tools_dirs_shape() {
    let (dir, _g) = scoped();
    let root = dir.path();
    // Skills pool + tools pools.
    make_resource(root, "skills", "core", 2, &[("git/SKILL.md", "git")]);
    make_resource(root, "tools", "zeta", 1, &[("run.sh", "")]);
    make_resource(root, "tools", "alpha", 3, &[("fmt", "")]);
    make_resource(root, "tools", "noversion", 0, &[]); // current 0 ⇒ skipped
                                                       // Cards: full ref, stale ref, no ref.
    let card = |name: &str, skills: Option<&str>, tools: Option<&str>| {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("meta.json"),
            serde_json::json!({ "current": { "skills": skills, "tools": tools } }).to_string(),
        )
        .unwrap();
    };
    card("full", Some("core"), Some("zeta"));
    card("stale", Some("ghost"), Some("noversion"));
    card("bare", None, None);
    assert_eq!(
        agent_skill_roots("full"),
        vec![root.join("skills").join("core").join("v2")]
    );
    assert_eq!(
        agent_tools_dirs("full"),
        vec![root.join("tools").join("zeta").join("v1")]
    );
    assert!(agent_skill_roots("stale").is_empty());
    assert!(agent_tools_dirs("stale").is_empty());
    assert!(agent_skill_roots("bare").is_empty());
    assert!(agent_skill_roots("ghost").is_empty());
    // Active-agent wrappers follow the marker; none active ⇒ empty.
    assert!(active_skill_roots().is_empty() && active_tools_dirs().is_empty());
    set_active_agent(Some("full")).unwrap();
    assert_eq!(active_skill_roots(), agent_skill_roots("full"));
    assert_eq!(active_tools_dirs(), agent_tools_dirs("full"));
    // Union surface for ToolsScope::All: every current tools dir, sorted.
    assert_eq!(
        all_tools_dirs(),
        vec![
            root.join("tools").join("alpha").join("v3"),
            root.join("tools").join("zeta").join("v1"),
        ]
    );
}
