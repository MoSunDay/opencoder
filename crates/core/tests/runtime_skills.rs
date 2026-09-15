//! A dedicated test process keeps the production OnceLock isolated.
use std::fs;

#[test]
fn runtime_skill_discovery_stays_pinned_after_shared_skills_change() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("shared");
    fs::create_dir_all(source.join("custom")).unwrap();
    let original = "---\nname: isolated-handoff-skill\ndescription: release fixture\n---\noriginal";
    fs::write(source.join("custom/SKILL.md"), original).unwrap();
    let data = dir.path().join("r1");
    opencoder_core::skill::pin_runtime_skills(&data, Some(&source)).unwrap();
    let pinned = opencoder_core::skills_dir().unwrap();
    assert_eq!(pinned, data.canonicalize().unwrap().join("global-skills"));
    fs::write(source.join("custom/SKILL.md"), "new release").unwrap();
    opencoder_core::skill::pin_runtime_skills(&data, Some(&source)).unwrap();
    assert_eq!(
        fs::read_to_string(pinned.join("custom/SKILL.md")).unwrap(),
        original
    );
    let skills = opencoder_core::discover_skills();
    let skill = skills
        .iter()
        .find(|skill| skill.name == "isolated-handoff-skill")
        .unwrap();
    assert_eq!(skill.body.trim(), "original");
    assert!(skill.source.starts_with(&pinned));
    assert!(
        opencoder_core::skill::pin_runtime_skills(&dir.path().join("r2"), Some(&source)).is_err()
    );
    assert_eq!(opencoder_core::skills_dir().unwrap(), pinned);
}
