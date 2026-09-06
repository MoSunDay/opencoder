use super::*;

fn scoped_agents() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = meta::tests::OVERRIDE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    meta::set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

/// Write `<cat>/<name>/meta.json` pointing `current` at `v`.
fn write_resource_meta(root: &std::path::Path, cat: &str, name: &str, v: u32) {
    std::fs::create_dir_all(root.join(cat).join(name)).unwrap();
    std::fs::write(
        root.join(cat).join(name).join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": {v}, "history": [{v}] }}"#),
    )
    .unwrap();
}

/// Write one shared prompt pack: `prompts/<pname>/v{v}/` with the
/// given sections plus its pool `meta.json` (`current: v`).
fn write_prompt_pack(
    root: &std::path::Path,
    pname: &str,
    v: u32,
    soul: Option<&str>,
    how: Option<&str>,
    output: Option<&str>,
) {
    let vdir = root.join("prompts").join(pname).join(format!("v{v}"));
    std::fs::create_dir_all(&vdir).unwrap();
    for (file, body) in [("soul", soul), ("how", how), ("output", output)] {
        if let Some(b) = body {
            std::fs::write(vdir.join(format!("{file}.md")), b).unwrap();
        }
    }
    write_resource_meta(root, "prompts", pname, v);
}

/// Write one agent reference card naming pool resources (`None` = no
/// reference for that category).
fn write_agent_card(
    root: &std::path::Path,
    name: &str,
    prompt: Option<&str>,
    skills: Option<&str>,
    tools: Option<&str>,
    memory: Option<&str>,
) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    let meta = serde_json::json!({
        "name": name,
        "current": { "prompt": prompt, "skills": skills, "tools": tools, "memory": memory },
    });
    std::fs::write(root.join(name).join("meta.json"), meta.to_string()).unwrap();
}

/// Minimal resolvable file agent: a private shared pool named after
/// the agent (`prompts/<name>` v1) plus a card referencing it. Each
/// call gets its OWN pack so section leftovers never leak between
/// fixtures (write paths only add files, never clear them).
fn write_file_agent(
    root: &std::path::Path,
    name: &str,
    soul: Option<&str>,
    how: Option<&str>,
    output: Option<&str>,
) {
    write_prompt_pack(root, name, 1, soul, how, output);
    write_agent_card(root, name, Some(name), None, None, None);
}

/// A file agent resolves with composed prompt, Act/Primary kind, the
/// first soul line as description, and `ToolFilter::All`.
#[test]
fn file_agent_resolves_from_current_prompt_version() {
    let (dir, _g) = scoped_agents();
    write_file_agent(
        dir.path(),
        "myagent",
        Some("Identity: a lean rust reviewer.\nMore soul."),
        Some("How: read, then patch."),
        Some("Output: patch + tests."),
    );
    let a = resolve_agent("myagent").expect("file agent must resolve");
    assert_eq!(a.name, "myagent");
    assert_eq!(a.kind, AgentKind::Act);
    assert!(a.is_primary());
    assert_eq!(a.description, "Identity: a lean rust reviewer.");
    assert_eq!(
        a.prompt,
        "# Soul\nIdentity: a lean rust reviewer.\nMore soul.\n\n# How\nHow: read, then patch.\n\n# Output\nOutput: patch + tests."
    );
    assert!(a.tools.allows("bash") && a.tools.allows("anything"));
    // No soul file ⇒ the description falls back to a stable label.
    write_file_agent(dir.path(), "howonly", None, Some("how body"), None);
    let h = resolve_agent("howonly").expect("how-only agent resolves");
    assert_eq!(h.description, "Custom agent howonly");
}

/// The shared-pool property: two agents referencing the SAME prompt
/// name both resolve through it, and bumping the pool's `current` to
/// v2 (new version dir + meta, written by hand) updates both at once.
#[test]
fn shared_prompt_pool_two_agents_share_and_bump() {
    let (dir, _g) = scoped_agents();
    let root = dir.path();
    write_prompt_pack(root, "shared", 1, Some("v1 soul"), None, None);
    write_agent_card(root, "one", Some("shared"), None, None, None);
    write_agent_card(root, "two", Some("shared"), None, None, None);
    for name in ["one", "two"] {
        assert_eq!(
            resolve_agent(name).unwrap().prompt,
            "# Soul\nv1 soul",
            "{name} must resolve through the shared pack"
        );
    }
    // Hand-write the v2 bump: new version dir + updated pool meta.
    let v2 = root.join("prompts").join("shared").join("v2");
    std::fs::create_dir_all(&v2).unwrap();
    std::fs::write(v2.join("soul.md"), "v2 soul").unwrap();
    std::fs::write(
        root.join("prompts").join("shared").join("meta.json"),
        r#"{ "name": "shared", "current": 2, "history": [1, 2] }"#,
    )
    .unwrap();
    for name in ["one", "two"] {
        assert_eq!(
            resolve_agent(name).unwrap().prompt,
            "# Soul\nv2 soul",
            "{name} must see the v2 bump"
        );
    }
}

/// Reference-card degrade paths: no prompt ref ⇒ not resolvable; a
/// prompt ref to a missing resource (or one with `current: 0`, or a
/// current version dir that does not exist) ⇒ stale degrade to `None`.
#[test]
fn agent_without_prompt_ref_or_stale_refs_degrade() {
    let (dir, _g) = scoped_agents();
    let root = dir.path();
    // Card with only non-prompt refs ⇒ no prompt ⇒ None.
    write_agent_card(root, "noprompt", None, Some("s"), None, None);
    assert!(resolve_agent("noprompt").is_none());
    // Prompt ref to a resource that does not exist ⇒ None.
    write_agent_card(root, "stale", Some("ghost"), None, None, None);
    assert!(resolve_agent("stale").is_none());
    // Pool exists but has no version yet (current: 0) ⇒ None.
    write_resource_meta(root, "prompts", "noversion", 0);
    write_agent_card(root, "zero", Some("noversion"), None, None, None);
    assert!(resolve_agent("zero").is_none());
    // Pool meta points at a version dir that is missing ⇒ None.
    write_resource_meta(root, "prompts", "dangling", 3);
    write_agent_card(root, "dangling", Some("dangling"), None, None, None);
    assert!(resolve_agent("dangling").is_none());
}

/// Memory reference: a resolving `current.memory` ref appends a
/// `# Memory` section (trimmed body) to the composed prompt; no ref,
/// or a stale ref, appends nothing.
#[test]
fn memory_section_appended_when_ref_resolves() {
    let (dir, _g) = scoped_agents();
    let root = dir.path();
    write_prompt_pack(root, "default", 1, Some("soul body"), None, None);
    write_resource_meta(root, "memory", "longterm", 1);
    let memdir = root.join("memory").join("longterm").join("v1");
    std::fs::create_dir_all(&memdir).unwrap();
    std::fs::write(memdir.join("memory.md"), "  prefers small commits.  \n\n").unwrap();
    write_agent_card(
        root,
        "withmem",
        Some("default"),
        None,
        None,
        Some("longterm"),
    );
    write_agent_card(root, "nomem", Some("default"), None, None, None);
    write_agent_card(root, "stalemem", Some("default"), None, None, Some("ghost"));
    let with = resolve_agent("withmem").unwrap().prompt;
    assert!(with.ends_with("# Soul\nsoul body\n\n# Memory\nprefers small commits."));
    assert!(!resolve_agent("nomem").unwrap().prompt.contains("# Memory"));
    assert!(
        !resolve_agent("stalemem")
            .unwrap()
            .prompt
            .contains("# Memory"),
        "a stale memory ref must append nothing"
    );
}

/// Missing files degrade: sections are optional, but an agent with no
/// readable section at all (missing, or a prompt version that does not
/// exist, or no prompt reference in the card) is not a real agent →
/// `None`.
#[test]
fn file_agent_missing_sections_and_versions_degrade() {
    let (dir, _g) = scoped_agents();
    write_file_agent(dir.path(), "partial", None, Some("only how"), None);
    assert_eq!(resolve_agent("partial").unwrap().prompt, "# How\nonly how");
    // Pool version exists but holds no section files.
    write_prompt_pack(dir.path(), "bare", 1, None, None, None);
    write_agent_card(dir.path(), "empty", Some("bare"), None, None, None);
    assert!(resolve_agent("empty").is_none());
    // Blank-only sections are as good as missing.
    write_file_agent(dir.path(), "blank", Some("  "), None, None);
    assert!(resolve_agent("blank").is_none());
}

/// Builtin names always win over same-named file agents, and corrupt
/// file agents fall back to `None` (builtin-fallback behavior) without
/// touching traversal paths.
#[test]
fn builtin_wins_and_corrupt_file_agents_fall_back() {
    let (dir, _g) = scoped_agents();
    // A file agent (illegally) named like a builtin must not shadow it.
    write_file_agent(dir.path(), "act", Some("impostor"), None, None);
    assert!(resolve_agent("act")
        .unwrap()
        .prompt
        .contains("You are OpenCoder"));
    // Corrupt agent card → silent None.
    write_file_agent(dir.path(), "corrupt", Some("s"), None, None);
    std::fs::write(dir.path().join("corrupt/meta.json"), "{ not json").unwrap();
    assert!(resolve_agent("corrupt").is_none());
    // Corrupt PROMPT POOL meta → the shared resource degrades → None.
    write_agent_card(dir.path(), "badpool", Some("broken"), None, None, None);
    std::fs::create_dir_all(dir.path().join("prompts").join("broken")).unwrap();
    std::fs::write(dir.path().join("prompts/broken/meta.json"), "{ not json").unwrap();
    assert!(resolve_agent("badpool").is_none());
    // Path safety: traversal strings never reach the filesystem layer.
    assert!(resolve_agent("../evil").is_none());
    assert!(resolve_agent("a/b").is_none());
    assert!(resolve_agent("").is_none());
}

/// All four tiers of [`effective_default_agent`]: CLI > active file
/// agent > config default > `"act"`.
#[test]
fn effective_default_agent_priority_tiers() {
    let (dir, _g) = scoped_agents();
    let cfg = Config::default();
    // Tier 4: nothing set anywhere.
    assert_eq!(effective_default_agent(None, &cfg), "act");
    // Tier 3: config default (non-empty) beats "act".
    let mut cfg_d = Config::default();
    cfg_d.agent.default = "plan".into();
    assert_eq!(effective_default_agent(None, &cfg_d), "plan");
    // Blank config default is skipped → tier 4.
    let mut cfg_blank = Config::default();
    cfg_blank.agent.default = "  ".into();
    assert_eq!(effective_default_agent(None, &cfg_blank), "act");
    // Tier 2: the active file agent beats config.
    write_file_agent(dir.path(), "mine", Some("soul line"), None, None);
    meta::set_active_agent(Some("mine")).unwrap();
    assert_eq!(effective_default_agent(None, &cfg_d), "mine");
    // A stale marker deactivates silently → tier 3 again.
    std::fs::remove_dir_all(dir.path().join("mine")).unwrap();
    assert_eq!(effective_default_agent(None, &cfg_d), "plan");
    // Tier 1: the CLI override beats everything (and blank is skipped).
    assert_eq!(effective_default_agent(Some("cli"), &cfg_d), "cli");
    assert_eq!(effective_default_agent(Some("  "), &cfg_d), "plan");
}
