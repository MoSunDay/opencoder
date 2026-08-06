    use super::{is_suspicious_model, scoped_config_home, Config};

    #[test]
    fn empty_model_is_not_suspicious() {
        assert!(!is_suspicious_model(""));
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
    fn theme_defaults_to_dark() {
        assert_eq!(Config::default().theme, "dark");
    }

    #[test]
    fn has_editable_key_recognizes_theme() {
        let v = serde_json::json!({ "theme": "light" });
        assert!(super::merge::has_editable_key(&v));
    }

    #[test]
    fn has_editable_key_recognizes_enable_tmux_session() {
        let v = serde_json::json!({ "enable_tmux_session": true });
        assert!(super::merge::has_editable_key(&v));
    }

    #[test]
    fn merge_into_applies_theme() {
        let mut c = Config::default();
        super::merge::merge_into(
            &mut c,
            serde_json::json!({ "theme": "light", "model": "openai/gpt-4o" }),
        );
        assert_eq!(c.theme, "light");
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
        let corrupt_res = Config::save(dir.path(), &serde_json::json!({ "theme": "light" }));
        let corrupt_contents = std::fs::read_to_string(&target).unwrap();

        // Empty/whitespace file: treated as an empty object, patch applied.
        std::fs::write(&target, "   \n  ").unwrap();
        let empty_res = Config::save(dir.path(), &serde_json::json!({ "theme": "light" }));
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
        assert_eq!(written["theme"], "light");
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
        super::merge::merge_into(
            &mut c,
            serde_json::json!({ "enable_tmux_session": true }),
        );
        assert_eq!(c.enable_tmux_session, Some(true));
    }
