//! Unit tests for [`super`] (version + reference-card writes), split out
//! of `write.rs` to respect the file line budget; mirrors the
//! `agent::meta::tests` conventions.

use super::*;
use crate::testutil::scoped;

fn vf(rel: &str) -> VersionFile {
    VersionFile {
        rel_path: rel.into(),
        bytes: rel.as_bytes().to_vec(),
    }
}

fn meta_of(cat: &str, name: &str) -> ResourceMeta {
    read_resource_meta(cat, name).unwrap()
}

#[test]
fn versions_increment_and_never_reuse() {
    let (tmp, _g) = scoped();
    assert_eq!(
        save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
        1
    );
    assert_eq!(
        save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
        2
    );
    assert_eq!(
        save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
        3
    );
    crate::rollback::rollback_resource("prompts", "pack", 1).unwrap();
    assert_eq!(
        save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
        4
    );
    let meta = meta_of("prompts", "pack");
    assert_eq!(meta.current, 4);
    assert_eq!(meta.history, vec![1, 2, 3, 4]);
    for v in 1..=4 {
        assert!(tmp.path().join(format!("prompts/pack/v{v}")).is_dir());
    }
    // Unknown category rejected before touching the fs.
    assert_eq!(
        save_resource_version("nope", "pack", &[vf("x")])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn failed_save_leaves_no_temp_and_meta_unchanged() {
    let (tmp, _g) = scoped();
    save_resource_version("tools", "kit", &[vf("run.sh")]).unwrap();
    let before = meta_of("tools", "kit");
    let err = save_resource_version("tools", "kit", &[vf("../escape")]).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    let kit = tmp.path().join("tools/kit");
    let temps: Vec<_> = std::fs::read_dir(&kit)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
        .collect();
    assert!(temps.is_empty(), "temp dirs left behind: {temps:?}");
    assert_eq!(meta_of("tools", "kit"), before);
    // Empty and absolute rel_paths are rejected too.
    assert!(save_resource_version("tools", "kit", &[vf("")]).is_err());
    assert!(save_resource_version("tools", "kit", &[vf("/etc/x")]).is_err());
}

#[test]
fn create_update_card_history() {
    let (_tmp, _g) = scoped();
    save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap();
    save_resource_version("tools", "kit", &[vf("run.sh")]).unwrap();
    let first = AgentRefs {
        prompt: Some("pack".into()),
        skills: None,
        tools: None,
        memory: None,
    };
    create_agent("work", first.clone()).unwrap();
    let card = read_agent_meta("work").unwrap();
    assert!(card.history.is_empty());
    assert_eq!(card.current, first);
    assert_eq!(card.references.prompt_files, vec!["soul"]);
    // Duplicate create rejected.
    assert_eq!(
        create_agent("work", Default::default()).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    // Change two fields → exactly two history entries.
    update_agent_refs(
        "work",
        AgentRefs {
            prompt: Some("pack".into()),
            skills: None,
            tools: Some("kit".into()),
            memory: Some("bank".into()),
        },
    )
    .unwrap();
    let card = read_agent_meta("work").unwrap();
    let fields: Vec<&str> = card.history.iter().map(|h| h.field.as_str()).collect();
    assert_eq!(fields, vec!["tools", "memory"]);
    assert_eq!(card.history[0].from, None);
    assert_eq!(card.history[0].to.as_deref(), Some("kit"));
    assert_eq!(card.references.tools, vec!["run.sh"]);
    // Identical refs → no new history entries.
    update_agent_refs("work", card.current.clone()).unwrap();
    assert_eq!(read_agent_meta("work").unwrap().history.len(), 2);
    // Unknown card rejected.
    assert_eq!(
        update_agent_refs("ghost", Default::default())
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    // Invalid names rejected. `active` is no longer reserved: the global
    // activation marker is gone, so it is just a regular card name.
    assert!(create_agent("../x", Default::default()).is_err());
}

#[test]
fn delete_agent_is_idempotent() {
    let (tmp, _g) = scoped();
    create_agent("gone", Default::default()).unwrap();
    assert!(tmp.path().join("gone").is_dir());
    delete_agent("gone").unwrap();
    delete_agent("gone").unwrap();
    assert!(!tmp.path().join("gone").exists());
    assert!(delete_agent("never-there").is_ok());
}

#[test]
fn profile_updates_preserve_resources_without_runtime_selection() {
    let (_tmp, _g) = scoped();
    create_agent_with_profile(
        "reviewer",
        Default::default(),
        opencoder_core::harness::Harness::Codex,
        Some("first".into()),
    )
    .unwrap();
    let card = read_agent_meta("reviewer").unwrap();
    assert_eq!(card.harness_profile.as_deref(), Some("first"));
    update_agent_with_profile("reviewer", None, None, Some(Some("second".into()))).unwrap();
    let card = read_agent_meta("reviewer").unwrap();
    assert_eq!(card.harness_profile.as_deref(), Some("second"));
    assert_eq!(card.history.len(), 1);
    assert_eq!(card.history[0].field, "harness_profile");
    assert!(serde_json::to_value(card)
        .unwrap()
        .get("run_mode")
        .is_none());
    assert!(create_agent_with_profile(
        "invalid",
        Default::default(),
        Default::default(),
        Some("codex-profile".into())
    )
    .is_err());
}
