//! Unit tests for `share_fs`: path safety, atomic writes, listing, tool-ref
//! resolution and the root override chain. The process-global override is
//! guarded by a mutex so parallel tests never race each other's root.

use std::path::PathBuf;
use std::sync::MutexGuard;
use std::sync::{Mutex, OnceLock};

fn gate() -> MutexGuard<'static, ()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    let m = GATE.get_or_init(|| Mutex::new(()));
    // Poisoned lock (earlier panic) must not cascade: re-acquire regardless.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_root() -> (MutexGuard<'static, ()>, PathBuf) {
    let g = gate();
    let root = std::env::temp_dir().join(format!("oc-share-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    crate::share_fs::set_share_dir_override(Some(root.clone()));
    (g, root)
}

#[test]
fn validate_share_name_rejects_traversal_and_separators() {
    assert!(crate::share_fs::validate_share_name("ffmpeg-v3").is_ok());
    assert!(crate::share_fs::validate_share_name("a").is_ok());
    for bad in ["", ".", "..", "a/b", "a\\b", "a\0b", &"x".repeat(129)] {
        assert!(
            crate::share_fs::validate_share_name(bad).is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn path_constructors_reject_bad_parts() {
    let root = PathBuf::from("/share");
    assert!(crate::share_fs::todo_version_dir(&root, "../escape", "v1").is_err());
    assert!(crate::share_fs::env_context_path(&root, "x/y").is_err());
    assert!(crate::share_fs::agent_tool_path(&root, "v1", "pwn/../../etc").is_err());
    assert!(crate::share_fs::todo_dir(&root, "ok").is_ok());
}

#[test]
fn atomic_write_json_roundtrip_and_no_tmp_leftover() {
    let (_g, root) = with_root();
    let path = root.join("todo/t1/todo.json");
    let value = serde_json::json!({"name": "t1", "current": "v1"});
    crate::share_fs::atomic_write_json(&path, &value).unwrap();
    assert_eq!(crate::share_fs::read_json_opt(&path).unwrap(), Some(value));
    let leftovers: Vec<_> = std::fs::read_dir(root.join("todo/t1"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "rename must not leave tmp files");
    // Missing file → None, not an error.
    assert_eq!(
        crate::share_fs::read_json_opt(&root.join("todo/t1/absent.json")).unwrap(),
        None
    );
}

#[test]
fn list_children_separates_dirs_and_files_sorted() {
    let (_g, root) = with_root();
    crate::share_fs::atomic_write(&root.join("env/b/env.json"), b"{}").unwrap();
    crate::share_fs::atomic_write(&root.join("env/a/env.json"), b"{}").unwrap();
    std::fs::create_dir_all(root.join("env/c")).unwrap();
    assert_eq!(
        crate::share_fs::list_child_dirs(&root.join("env")),
        vec!["a", "b", "c"]
    );
    assert_eq!(
        crate::share_fs::list_child_files(&root.join("env/a")),
        vec!["env.json"]
    );
    assert!(crate::share_fs::list_child_dirs(&root.join("nope")).is_empty());
}

#[test]
fn resolve_tool_ref_validates_shape_and_existence() {
    let (_g, root) = with_root();
    crate::share_fs::atomic_write(&root.join("agent/tools/v3/ffmpeg"), b"#!/bin/sh\n").unwrap();
    let ok = crate::share_fs::resolve_tool_ref(&root, "/agent/tools/v3/ffmpeg");
    assert_eq!(ok.unwrap(), root.join("agent/tools/v3/ffmpeg"));
    for bad in [
        "/agent/tools/v3/missing",
        "/agent/tools/v3/ffmpeg/extra",
        "/agent/tools//ffmpeg",
        "agent/tools/v3/ffmpeg",
        "/etc/passwd",
        "/agent/tools/../v3/ffmpeg",
    ] {
        assert!(
            crate::share_fs::resolve_tool_ref(&root, bad).is_err(),
            "{bad:?} must not resolve"
        );
    }
}

#[test]
fn override_beats_env_and_config_in_resolution() {
    let (_g, root) = with_root();
    assert_eq!(
        crate::share_fs::effective_share_dir(None),
        Some(root.clone())
    );
    // Config participates when no override/env is set.
    crate::share_fs::set_share_dir_override(None);
    let cfg = crate::Config {
        agent: crate::AgentDefaults {
            share_dir: Some(PathBuf::from("/cfg/share")),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        crate::share_fs::effective_share_dir(Some(&cfg)),
        Some(PathBuf::from("/cfg/share"))
    );
    assert!(crate::share_fs::effective_share_dir(None).is_some());
}
