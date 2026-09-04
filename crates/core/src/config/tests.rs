use super::{is_suspicious_model, scoped_config_home, Config};

#[test]
fn empty_model_is_suspicious() {
    // Empty model resolves to a request with `model: ""` — every call would
    // fail; treat it as malformed so `Config::save` refuses to persist it.
    assert!(is_suspicious_model(""));
}

#[test]
fn well_formed_scoped_model_is_not_suspicious() {
    assert!(!is_suspicious_model("openai/gpt-4o"));
    assert!(!is_suspicious_model("anthropic/claude-3.5-sonnet"));
}

#[test]
fn boundary_two_char_sides_are_not_suspicious() {
    // provider.len() == 2 && mid.len() == 2 is the minimum valid scoped model
    assert!(!is_suspicious_model("ab/cd"));
}

#[test]
fn unscoped_short_model_is_suspicious() {
    assert!(is_suspicious_model("x")); // len < 3, no slash
}

#[test]
fn unscoped_three_char_model_is_not_suspicious() {
    // len == 3 is the minimum valid unscoped model (boundary)
    assert!(!is_suspicious_model("abc"));
}

#[test]
fn short_provider_side_is_suspicious() {
    assert!(is_suspicious_model("a/bc")); // provider.len() < 2
}

#[test]
fn short_model_side_is_suspicious() {
    assert!(is_suspicious_model("ab/c")); // mid.len() < 2
}

#[test]
fn stream_idle_timeout_defaults_to_600s() {
    assert_eq!(
        Config::default().stream_idle_timeout(),
        std::time::Duration::from_secs(600)
    );
}

#[test]
fn stream_idle_timeout_is_configurable() {
    let c = Config {
        stream_idle_timeout_secs: Some(60),
        ..Default::default()
    };
    assert_eq!(c.stream_idle_timeout(), std::time::Duration::from_secs(60));
}

#[test]
fn task_timeout_defaults_to_1800s() {
    assert_eq!(
        Config::default().task_timeout(),
        std::time::Duration::from_secs(1800)
    );
}

#[test]
fn task_timeout_is_configurable() {
    let c = Config {
        task_timeout_secs: Some(300),
        ..Default::default()
    };
    assert_eq!(c.task_timeout(), std::time::Duration::from_secs(300));
}
#[test]
fn replay_timeout_defaults_to_300s() {
    assert_eq!(
        Config::default().replay_timeout(),
        std::time::Duration::from_secs(300)
    );
}

#[test]
fn replay_timeout_is_configurable() {
    let c = Config {
        replay_timeout_secs: Some(60),
        ..Default::default()
    };
    assert_eq!(c.replay_timeout(), std::time::Duration::from_secs(60));
}

#[test]
fn subagent_drain_defaults_to_15s() {
    assert_eq!(
        Config::default().subagent_drain(),
        std::time::Duration::from_secs(15)
    );
}

#[test]
fn subagent_drain_is_configurable() {
    let c = Config {
        subagent_drain_secs: Some(5),
        ..Default::default()
    };
    assert_eq!(c.subagent_drain(), std::time::Duration::from_secs(5));
}

#[test]
fn has_editable_key_recognizes_enable_tmux_session() {
    let v = serde_json::json!({ "enable_tmux_session": true });
    assert!(super::merge::has_editable_key(&v));
}

/// Every key `merge_into` applies but `has_editable_key` used to miss: a
/// config file carrying ONLY one of these keys must count as editable, or a
/// save routed through `save_target` would create a brand-new opencoder.json
/// instead of merging into the file the user already has.
#[test]
fn has_editable_key_recognizes_timeout_and_agent_keys() {
    for v in [
        serde_json::json!({ "stream_idle_timeout_secs": 60 }),
        serde_json::json!({ "task_timeout_secs": 300 }),
        serde_json::json!({ "replay_timeout_secs": 90 }),
        serde_json::json!({ "subagent_drain_secs": 5 }),
        serde_json::json!({ "agent": { "default": "act" } }),
        serde_json::json!({ "agent": { "unknown_future_key": 1 } }),
        serde_json::json!({ "output_streamline": { "enabled": false } }),
        serde_json::json!({ "tool_guard": { "max_consecutive_failures": 5 } }),
    ] {
        assert!(
            super::merge::has_editable_key(&v),
            "should be editable: {v}"
        );
    }
}

/// Negative side of the same coin: EMPTY object values (nothing merge_into
/// would apply) and unrelated keys stay non-editable.
#[test]
fn has_editable_key_ignores_empty_agent_and_object_keys() {
    for v in [
        serde_json::json!({ "agent": {} }),
        serde_json::json!({ "output_streamline": {} }),
        serde_json::json!({ "tool_guard": {} }),
        serde_json::json!({ "output_streamline": "not-an-object" }),
        serde_json::json!({ "totally_unrelated": 42 }),
    ] {
        assert!(
            !super::merge::has_editable_key(&v),
            "should NOT be editable: {v}"
        );
    }
}

#[test]
fn has_editable_key_ignores_domain_keys() {
    // config.json whose ONLY keys are the (hard-cut) domain keys is not an
    // editable config.json — those keys live in mcp.json / cli.json /
    // skills.json / ap.json and never route a save back into config.json.
    let v = serde_json::json!({
        "mcp_servers": { "srv": { "enabled": true } },
        "cli": { "git": { "enabled": true, "content": "c" } },
        "skills": { "review": { "enabled": true } },
        "autopilot": { "mode": "ap", "max_iterations": 5 }
    });
    assert!(!super::merge::has_editable_key(&v));
}

#[test]
fn merge_into_applies_model() {
    let mut c = Config::default();
    super::merge::merge_into(&mut c, serde_json::json!({ "model": "openai/gpt-4o" }));
    assert_eq!(c.model, "openai/gpt-4o");
}

// --- Bug 3: AgentDefaults serde default must agree with Default impl ---
#[test]
fn agent_defaults_empty_object_deserializes_to_act() {
    // Deserializing {} must match the Default impl ("act"), not "".
    let ad: super::AgentDefaults = serde_json::from_str("{}").unwrap();
    assert_eq!(ad.default, "act");
    assert_eq!(ad.default, super::AgentDefaults::default().default);
}

// --- Bug 1: Config::save must not silently wipe a corrupt config file ---
// HOME is isolated to the temp dir so `save_target` resolves entirely
// within it — otherwise a real ~/.opencoder/config.json carrying editable
// keys would shadow the temp file. Both cases share one test so there is a
// single HOME override, and HOME is restored before any assertion can panic.
#[test]
fn save_handles_corrupt_and_empty_config_files() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("opencoder.json");
    // Thread-local isolation (no process-env mutation): keeps `save_target`'s
    // global candidates inside the tempdir so a real ~/.opencoder/config.json
    // can't shadow it — without racing concurrent `dirs::data_local_dir()`
    // reads under parallel test execution (the setenv/getenv UB flake).
    let _home = scoped_config_home(dir.path().to_path_buf());

    // Corrupt file: save must refuse and leave it untouched.
    let corrupt = "{ this is :: not valid json";
    std::fs::write(&target, corrupt).unwrap();
    let corrupt_res = Config::save(dir.path(), &serde_json::json!({ "fps": 20 }));
    let corrupt_contents = std::fs::read_to_string(&target).unwrap();

    // Empty/whitespace file: treated as an empty object, patch applied.
    std::fs::write(&target, "   \n  ").unwrap();
    let empty_res = Config::save(dir.path(), &serde_json::json!({ "fps": 20 }));
    let empty_written: Option<serde_json::Value> = empty_res
        .ok()
        .and_then(|p| serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).ok());

    // `_home` restores the prior isolation state on drop (even on panic), so
    // no override leaks into the rest of the process.

    assert!(
        corrupt_res.is_err(),
        "save should refuse a corrupt file, got {corrupt_res:?}"
    );
    assert_eq!(
        corrupt_contents, corrupt,
        "corrupt file must be left untouched"
    );
    let written = empty_written.expect("save of an empty/whitespace file should succeed");
    assert_eq!(written["fps"], serde_json::json!(20));
}

// --- Bug 2: non-object config files are tolerated (warned, not errored) ---
#[test]
fn load_tolerates_non_object_config_file() {
    let dir = tempfile::tempdir().unwrap();
    // A valid-JSON-but-not-object candidate must not break load.
    std::fs::write(dir.path().join("opencoder.json"), "[1, 2, 3]").unwrap();
    let cfg = Config::load(dir.path());
    assert!(cfg.is_ok(), "load should not error on a non-object file");
}

// --- Bug 4: provider headers must be merged (appended), not replaced ---
#[test]
fn merge_into_appends_provider_headers() {
    let mut c = Config::default();
    c.providers.insert(
        "foo".into(),
        super::ProviderConfig {
            headers: vec![super::HttpHeader {
                name: "X-Global".into(),
                value: "1".into(),
            }],
            ..Default::default()
        },
    );
    super::merge::merge_into(
        &mut c,
        serde_json::json!({
            "providers": {
                "foo": {
                    "headers": [{ "name": "X-Project", "value": "2" }]
                }
            }
        }),
    );
    let headers = &c.providers["foo"].headers;
    assert_eq!(headers.len(), 2, "project headers should append to global");
    assert_eq!(headers[0].name, "X-Global");
    assert_eq!(headers[1].name, "X-Project");
}

#[test]
fn enable_tmux_session_defaults_to_none() {
    assert!(Config::default().enable_tmux_session.is_none());
}

#[test]
fn merge_into_applies_enable_tmux_session() {
    let mut c = Config::default();
    super::merge::merge_into(&mut c, serde_json::json!({ "enable_tmux_session": true }));
    assert_eq!(c.enable_tmux_session, Some(true));
}

// --- Skill default-injection toggles (`/skill` menu) ---
#[test]
fn skills_default_empty_and_enabled_names_follow_toggles() {
    let empty: Config = serde_json::from_str("{}").unwrap();
    assert!(empty.skills.is_empty());
    assert!(empty.enabled_skill_names().is_empty());

    let cfg: Config =
        serde_json::from_str(r#"{"skills":{"review":{"enabled":true},"other":{}}}"#).unwrap();
    // missing `enabled` deserializes as false (Default OFF)
    assert!(!cfg.skills["other"].enabled);
    assert_eq!(cfg.enabled_skill_names(), vec!["review".to_string()]);
}

#[test]
fn merge_into_preserves_siblings_and_hard_cuts_domain_keys() {
    let mut c = Config::default();
    c.compaction.tail_turns = 5;
    super::merge::merge_into(
        &mut c,
        // a legacy config.json still carrying the domain keys alongside a
        // real config key: the domain keys are ignored (hard-cut), the rest
        // merges normally with sub-key sibling preservation.
        serde_json::json!({
            "compaction": { "context_threshold": 9000 },
            "skills": { "deploy": { "enabled": true } },
            "mcp_servers": { "srv": { "enabled": true } },
            "cli": { "git": { "enabled": true, "content": "c" } }
        }),
    );
    // entry-level merge: the pre-existing `tail_turns` survives the patch
    assert_eq!(c.compaction.tail_turns, 5);
    assert_eq!(c.compaction.context_threshold, 9000);
    // domain keys are hard-cut out of config.json (they live in domain files)
    assert!(c.skills.is_empty(), "legacy config.json `skills` ignored");
    assert!(
        c.mcp_servers.is_empty(),
        "legacy config.json `mcp_servers` ignored"
    );
    assert!(c.cli.is_empty(), "legacy config.json `cli` ignored");
}

#[test]
fn enabled_skill_names_are_sorted() {
    let cfg: Config = serde_json::from_str(
        r#"{"skills":{"zeta":{"enabled":true},"alpha":{"enabled":true},"mid":{"enabled":true}}}"#,
    )
    .unwrap();
    assert_eq!(
        cfg.enabled_skill_names(),
        vec!["alpha".to_string(), "mid".to_string(), "zeta".to_string()]
    );
}

#[test]
fn load_reads_skills_from_domain_file() {
    let dir = tempfile::tempdir().unwrap();
    // Thread-local HOME isolation (see save_handles_corrupt_and_empty_config_files)
    // so a real global skills.json can't leak entries into this load.
    let _home = scoped_config_home(dir.path().to_path_buf());
    std::fs::create_dir_all(dir.path().join(".opencoder")).unwrap();
    // domain file = the bare entries map (no `skills` envelope — the file IS
    // the domain)
    std::fs::write(
        dir.path().join(".opencoder").join("skills.json"),
        r#"{"review":{"enabled":true},"other":{"enabled":false}}"#,
    )
    .unwrap();
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.skills.len(), 2);
    assert!(cfg.skills["review"].enabled);
    assert!(!cfg.skills["other"].enabled);
    assert_eq!(cfg.enabled_skill_names(), vec!["review".to_string()]);
}

#[cfg(test)]
mod inject_to_filtering {
    use super::super::{CliConfig, Config, McpServerConfig};
    use crate::AgentMode;
    use crate::InjectionTarget;

    fn config_with(target: InjectionTarget) -> Config {
        let mut config = Config::default();
        config.cli.insert(
            "probe".into(),
            CliConfig {
                enabled: true,
                inject_to: target,
                content: "c".into(),
            },
        );
        config.mcp_servers.insert(
            "probe".into(),
            McpServerConfig {
                enabled: true,
                inject_to: target,
                ..Default::default()
            },
        );
        config
    }

    #[test]
    fn enabled_for_filters_by_agent_name_within_subagents() {
        let explore_only = InjectionTarget {
            parent: false,
            explore: true,
            build: false,
        };
        let config = config_with(explore_only);
        assert_eq!(
            config.enabled_cli_for("explore", AgentMode::Subagent).len(),
            1
        );
        assert_eq!(
            config.enabled_cli_for("build", AgentMode::Subagent).len(),
            0
        );
        assert_eq!(config.enabled_cli_for("act", AgentMode::Primary).len(), 0);

        let mut cli = config.enabled_mcp_servers_for("explore", AgentMode::Subagent);
        assert_eq!(cli.len(), 1);
        cli = config.enabled_mcp_servers_for("build", AgentMode::Subagent);
        assert!(cli.is_empty());
    }

    #[test]
    fn parent_flag_covers_every_primary_agent() {
        let config = config_with(InjectionTarget::parent_only());
        for name in ["act", "sandbox", "command", "workflow"] {
            assert_eq!(
                config.enabled_cli_for(name, AgentMode::Primary).len(),
                1,
                "{name} is a primary agent"
            );
            assert_eq!(
                config
                    .enabled_mcp_servers_for(name, AgentMode::Primary)
                    .len(),
                1
            );
        }
        assert!(config
            .enabled_mcp_servers_for("explore", AgentMode::Subagent)
            .is_empty());
    }

    #[test]
    fn legacy_subagents_value_loads_and_filters_to_both_subagents() {
        // A config written by an older build says "subagents".
        let json =
            r#"{"cli": {"old": {"enabled": true, "inject_to": "subagents", "content": "x"}}}"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let config = Config::default().merged_with(&value);
        assert_eq!(
            config.enabled_cli_for("explore", AgentMode::Subagent).len(),
            1
        );
        assert_eq!(
            config.enabled_cli_for("build", AgentMode::Subagent).len(),
            1
        );
        assert!(config.enabled_cli_for("act", AgentMode::Primary).is_empty());
    }

    #[test]
    fn legacy_all_value_loads_into_every_agent() {
        let json = r#"{"cli": {"old": {"enabled": true, "inject_to": "all", "content": "x"}}}"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let config = Config::default().merged_with(&value);
        assert_eq!(config.enabled_cli_for("act", AgentMode::Primary).len(), 1);
        assert_eq!(
            config.enabled_cli_for("explore", AgentMode::Subagent).len(),
            1
        );
        assert_eq!(
            config.enabled_cli_for("build", AgentMode::Subagent).len(),
            1
        );
    }
}

// --- MCP server name-collision guard (bug #14, save-time net) ---
// `Config::save` must refuse to persist two `mcp_servers` entries whose
// names normalize (`[-.]` -> `_`) to the same tool prefix: registration
// would silently shadow one server's tools with the other's. The TUI form
// pre-checks this (`crates/tui/src/mcp_menu/patch.rs::colliding_server`);
// these tests pin the core-level second net that also covers the web API.

/// All candidate mcp.json locations a `Config::save(working_dir, …)` may
/// write: the project file and the scoped-home global one. With the scoped
/// home set to `home`, the env layer is absent, so the write target is the
/// project file when it exists, else `<home>/mcp.json`.
fn mcp_json_candidates(
    working_dir: &std::path::Path,
    home: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    vec![
        working_dir.join(".opencoder").join("mcp.json"),
        home.join("mcp.json"),
    ]
}

#[test]
fn save_rejects_mcp_servers_normalized_collision() {
    let dir = tempfile::tempdir().unwrap();
    let _home = scoped_config_home(dir.path().to_path_buf());
    let res = Config::save(
        dir.path(),
        &serde_json::json!({ "mcp_servers": {
            "a-b": { "command": "npx", "args": ["s1"] },
            "a.b": { "command": "npx", "args": ["s2"] },
        }}),
    );
    let err = format!(
        "{}",
        res.expect_err("colliding mcp server names must be refused")
    );
    assert!(err.contains("a-b") && err.contains("a.b"), "err = {err}");
    assert!(err.contains("mcp__a_b__"), "normalized form missing: {err}");
    // Nothing half-written: no candidate mcp.json may carry either server.
    for path in mcp_json_candidates(dir.path(), dir.path()) {
        if path.exists() {
            let raw = std::fs::read_to_string(&path).unwrap();
            assert!(
                !raw.contains("a-b") && !raw.contains("a.b"),
                "{} polluted with refused servers: {raw}",
                path.display()
            );
        }
    }
}

#[test]
fn save_allows_rename_via_null_delete_marker() {
    let dir = tempfile::tempdir().unwrap();
    let _home = scoped_config_home(dir.path().to_path_buf());
    Config::save(
        dir.path(),
        &serde_json::json!({ "mcp_servers": { "a-b": { "command": "npx" } } }),
    )
    .expect("initial single server saves");
    // Rename `a-b` -> `a.b` in one patch: the null deletes the old key, so
    // the merged map holds one server on the shared normalized slot — Ok.
    Config::save(
        dir.path(),
        &serde_json::json!({ "mcp_servers": { "a.b": { "command": "npx" }, "a-b": null } }),
    )
    .expect("rename with null delete marker must not trip the guard");
    let cfg = Config::load(dir.path()).unwrap();
    assert_eq!(cfg.mcp_servers.len(), 1, "exactly one server after rename");
    assert!(cfg.mcp_servers.contains_key("a.b"));
}

#[test]
fn save_allows_intra_patch_rename_on_fresh_file() {
    let dir = tempfile::tempdir().unwrap();
    let _home = scoped_config_home(dir.path().to_path_buf());
    Config::save(
        dir.path(),
        &serde_json::json!({ "mcp_servers": { "a.b": { "command": "npx" }, "a-b": null } }),
    )
    .expect("single save whose null marker deletes a key that never existed");
    let cfg = Config::load(dir.path()).unwrap();
    assert!(cfg.mcp_servers.contains_key("a.b"));
    assert!(!cfg.mcp_servers.contains_key("a-b"));
}

#[test]
fn save_without_mcp_servers_key_is_unaffected() {
    let dir = tempfile::tempdir().unwrap();
    let _home = scoped_config_home(dir.path().to_path_buf());
    Config::save(dir.path(), &serde_json::json!({ "model": "prov/model" }))
        .expect("non-mcp patches must not trip the guard");
}

#[test]
fn embedding_model_id_defaults_to_openai_small() {
    let cfg = Config::default();
    assert_eq!(cfg.embedding_model, None);
    assert_eq!(cfg.embedding_model_id(), "text-embedding-3-small");
}

#[test]
fn embedding_model_id_returns_configured_value_stripping_prefix() {
    let cfg = Config {
        embedding_model: Some("openai/text-embedding-3-large".into()),
        ..Config::default()
    };
    assert_eq!(cfg.embedding_model_id(), "text-embedding-3-large");
    let cfg = Config {
        embedding_model: Some("bge-m3".into()),
        ..Config::default()
    };
    assert_eq!(cfg.embedding_model_id(), "bge-m3");
}
