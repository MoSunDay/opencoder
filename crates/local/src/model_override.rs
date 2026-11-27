//! `--model` override plumbing for the headless run / todos paths.
//!
//! Both entry points share one rule: an explicit `--model` must be a well
//! formed `provider/model` string. A malformed override (empty, one-sided, or
//! with a side shorter than 2 chars) is rejected with a user-facing error
//! BEFORE any endpoint/api-key resolution, so the user sees the actual
//! mistake instead of a confusing downstream auth failure (e2e E20d).

use opencoder_core::{config::is_suspicious_model, Config};
use opencoder_session::SessionState;

/// The shared error message for a malformed `--model` value. Wording is pinned
/// by e2e (`scripts/e2e/config_scenarios.py` E20d looks for "malformed" or
/// "provider/model" in stderr): keep both tokens.
pub(crate) fn invalid_model_error(m: &str) -> String {
    format!(
        "malformed --model value `{m}`: expected \"provider/model\" with each side at least 2 chars (e.g. \"openai/gpt-4o\")"
    )
}

/// Apply a `--model` override (format `provider/model_id`) to the config.
/// Must be called before `resolve_endpoint` so the LLM client is built against
/// the chosen provider's credentials. `Err` on a malformed value (never
/// silently applied); `Ok(true)` when the config changed.
pub(crate) fn apply_model_override(
    config: &mut Config,
    model: &Option<String>,
) -> std::result::Result<bool, String> {
    if let Some(m) = model {
        if is_suspicious_model(m) {
            return Err(invalid_model_error(m));
        }
        return Ok(config.apply_explicit_model(m));
    }
    Ok(false)
}

/// Select the single process-level model that must survive session resume.
/// Config loading already applied the environment value; applying the CLI value
/// afterwards establishes the same priority for endpoint construction. Keeping
/// only the winner here prevents a later environment reapply from undoing CLI.
pub(crate) fn preferred_explicit_model(cli: Option<&str>, env: Option<&str>) -> Option<String> {
    cli.or_else(|| env.filter(|model| !model.is_empty()))
        .map(str::to_string)
}

/// Re-apply the winning process-level model after resume. Existing rows persist
/// both `sessions.model` and their legacy/mode-aware metadata before this
/// returns; fresh rows remain lazy. `Err` on malformed values or storage errors.
pub(crate) async fn reapply_resume_model(
    session: &mut SessionState,
    model: &Option<String>,
) -> std::result::Result<Option<String>, String> {
    let m = match model.as_ref() {
        Some(m) => m,
        None => return Ok(None),
    };
    if is_suspicious_model(m) {
        return Err(invalid_model_error(m));
    }
    let changed = session
        .apply_explicit_model_and_persist(m)
        .await
        .map_err(|error| format!("failed to persist explicit model: {error:#}"))?;
    Ok(changed.then(|| m.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_model_override_sets_provider_model() {
        let mut cfg = Config::default();
        assert!(apply_model_override(&mut cfg, &Some("anthropic/claude-3".into())).unwrap());
        assert_eq!(cfg.model, "anthropic/claude-3");
        assert_eq!(cfg.provider_id(), "anthropic");
        assert_eq!(cfg.model_id(), "claude-3");
        // same value -> no change (Ok(false)); no override -> no change
        assert!(!apply_model_override(&mut cfg, &Some("anthropic/claude-3".into())).unwrap());
        let mut cfg2 = Config::default();
        let before = cfg2.model.clone();
        assert!(!apply_model_override(&mut cfg2, &None).unwrap());
        assert_eq!(cfg2.model, before);
    }

    #[test]
    fn apply_model_override_clears_configured_mode_models() {
        use opencoder_core::{ConfigMode, ModeConfig, ModesConfig};

        let model = "anthropic/claude-3";
        let mut cfg = Config {
            model: model.into(),
            modes: ModesConfig {
                shared: ModeConfig {
                    model: Some("shared/model".into()),
                    max_tokens: Some(200),
                    ..Default::default()
                },
                act: Some(ModeConfig {
                    model: Some("act/model".into()),
                    ..Default::default()
                }),
                plan: Some(ModeConfig {
                    model: Some("plan/model".into()),
                    ..Default::default()
                }),
            },
            ..Default::default()
        };

        assert!(apply_model_override(&mut cfg, &Some(model.into())).unwrap());
        for mode in [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan] {
            assert_eq!(cfg.effective_for_mode(mode).model, model);
        }
        assert_eq!(cfg.modes.shared.max_tokens, Some(200));
        assert!(!apply_model_override(&mut cfg, &Some(model.into())).unwrap());
    }

    #[test]
    fn apply_model_override_rejects_malformed_values() {
        for bad in ["", "x", "ab/c"] {
            let mut cfg = Config::default();
            let before = cfg.model.clone();
            let err = apply_model_override(&mut cfg, &Some(bad.into()))
                .expect_err("malformed --model must be rejected");
            assert!(
                err.contains("malformed") && err.contains("provider/model") && err.contains(bad),
                "error must name the malformed value {bad:?}: {err}"
            );
            assert_eq!(cfg.model, before, "rejected value must not be applied");
        }
        // Well-formed "prov/model" passes both gates and is applied.
        let mut cfg = Config::default();
        assert!(apply_model_override(&mut cfg, &Some("prov/model".into())).unwrap());
        assert_eq!(cfg.model, "prov/model");
    }

    #[test]
    fn process_model_selection_prefers_cli_and_ignores_empty_environment() {
        assert_eq!(
            preferred_explicit_model(Some("cli/model"), Some("env/model")).as_deref(),
            Some("cli/model")
        );
        assert_eq!(
            preferred_explicit_model(None, Some("env/model")).as_deref(),
            Some("env/model")
        );
        assert_eq!(preferred_explicit_model(None, Some("")), None);
        assert_eq!(
            preferred_explicit_model(Some(""), Some("env/model")).as_deref(),
            Some(""),
            "an explicitly malformed CLI value must reach existing validation"
        );
    }

    fn session_with_model(model: &str) -> SessionState {
        use opencoder_llm::{ChatStream, MockChatClient};
        use std::{path::PathBuf, sync::Arc};
        SessionState::new(
            "s1",
            opencoder_core::resolve_agent("act").unwrap(),
            Config {
                model: model.into(),
                ..Config::default()
            },
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            PathBuf::from("/tmp"),
        )
    }

    #[tokio::test]
    async fn resume_reapplies_environment_or_cli_winner() {
        use opencoder_core::{ModeConfig, ModesConfig};
        use opencoder_llm::{ChatStream, MockChatClient};
        use opencoder_store::{SessionMeta, SessionModelOverrides, Store, TursoStore};
        use std::sync::Arc;

        let store = Arc::new(TursoStore::open_memory().await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let base = Config {
            model: "provider/file".into(),
            modes: ModesConfig {
                act: Some(ModeConfig {
                    model: Some("provider/act".into()),
                    reasoning_effort: Some("high".into()),
                    max_tokens: Some(200),
                    context_limit: Some(6_000),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        for (id, cli, expected) in [
            ("local-env", None, "env/model"),
            ("local-cli", Some("cli/model"), "cli/model"),
        ] {
            store
                .create_session_with_model_overrides(
                    &SessionMeta {
                        id: id.into(),
                        agent: Some("act".into()),
                        model: Some("provider/stored".into()),
                        created_at: 1,
                        updated_at: 1,
                        ..Default::default()
                    },
                    Some(&SessionModelOverrides {
                        shared: None,
                        act: Some("provider/menu-act".into()),
                        plan: Some("provider/menu-plan".into()),
                    }),
                )
                .await
                .unwrap();
            let mut startup = base.clone();
            startup.apply_explicit_model("env/model");
            apply_model_override(&mut startup, &cli.map(str::to_string)).unwrap();
            let explicit = preferred_explicit_model(cli, Some("env/model"));
            let client = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
            let mut session = opencoder_session::resume(
                store.clone(),
                id,
                startup,
                client,
                dir.path().to_path_buf(),
            )
            .await
            .unwrap();
            reapply_resume_model(&mut session, &explicit).await.unwrap();
            assert_eq!(session.effective_model(), expected);
            assert_eq!(
                session.effective_config().reasoning_effort.as_deref(),
                Some("high")
            );
            assert_eq!(session.effective_config().max_tokens, Some(200));
            assert_eq!(session.effective_context_limit(), 6_000);
            assert_eq!(
                store.get_session_model_overrides(id).await.unwrap(),
                Some(SessionModelOverrides::default())
            );
        }
    }

    #[tokio::test]
    async fn fresh_preapplied_cli_model_does_not_write_or_materialize_a_row() {
        use opencoder_llm::{ChatStream, MockChatClient};
        use opencoder_store::{Store, TursoStore};
        use std::sync::Arc;

        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.apply_explicit_model("env/model");
        apply_model_override(&mut config, &Some("cli/model".into())).unwrap();
        let client = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
        let mut session = SessionState::new(
            "local-lazy",
            opencoder_core::resolve_agent("act").unwrap(),
            config,
            client,
            dir.path().to_path_buf(),
        )
        .with_store(store.clone());
        let explicit = preferred_explicit_model(Some("cli/model"), Some("env/model"));
        assert_eq!(
            reapply_resume_model(&mut session, &explicit).await.unwrap(),
            None
        );
        assert_eq!(session.effective_model(), "cli/model");
        assert!(store.get_session("local-lazy").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reapply_resume_model_overrides_stored_model() {
        let mut s = session_with_model("openai/gpt-4o-mini");
        let changed = reapply_resume_model(&mut s, &Some("anthropic/claude-3".into()))
            .await
            .unwrap();
        assert_eq!(changed.as_deref(), Some("anthropic/claude-3"));
        assert_eq!(s.model, "claude-3");
        assert_eq!(s.config.provider_id(), "anthropic");
        // no override -> no change, Ok(None)
        assert_eq!(reapply_resume_model(&mut s, &None).await.unwrap(), None);
    }

    #[tokio::test]
    async fn reapply_resume_model_rejects_malformed_values() {
        for bad in ["", "x", "ab/c"] {
            let mut s = session_with_model("openai/gpt-4o-mini");
            let err = reapply_resume_model(&mut s, &Some(bad.into()))
                .await
                .expect_err("malformed --model must never be silently applied");
            assert!(
                err.contains("malformed") && err.contains(bad),
                "error must name the malformed value {bad:?}: {err}"
            );
            assert_eq!(
                s.config.model, "openai/gpt-4o-mini",
                "rejected value must not touch the resumed session"
            );
        }
        // Well-formed "prov/model" still wins over the stored model.
        let mut s = session_with_model("openai/gpt-4o-mini");
        assert_eq!(
            reapply_resume_model(&mut s, &Some("prov/model".into()))
                .await
                .unwrap()
                .as_deref(),
            Some("prov/model")
        );
    }
}
