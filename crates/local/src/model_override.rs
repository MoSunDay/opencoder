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
        if config.model != *m {
            config.model = m.clone();
            return Ok(true);
        }
    }
    Ok(false)
}

/// Re-apply an explicit `--model` to a resumed session. `resume()` restores
/// the stored model into the session, so an explicit `--model` must win here.
/// `Err` on a malformed value (never silently applied); `Ok(Some(new_model))`
/// when the session changed (caller persists it), else `Ok(None)`.
pub(crate) fn reapply_resume_model(
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
    if session.config.model == *m {
        return Ok(None);
    }
    session.config.model = m.clone();
    session.model = session.config.model_id().to_string();
    Ok(Some(m.clone()))
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

    #[test]
    fn reapply_resume_model_overrides_stored_model() {
        let mut s = session_with_model("openai/gpt-4o-mini");
        let changed = reapply_resume_model(&mut s, &Some("anthropic/claude-3".into())).unwrap();
        assert_eq!(changed.as_deref(), Some("anthropic/claude-3"));
        assert_eq!(s.model, "claude-3");
        assert_eq!(s.config.provider_id(), "anthropic");
        // no override -> no change, Ok(None)
        assert_eq!(reapply_resume_model(&mut s, &None).unwrap(), None);
    }

    #[test]
    fn reapply_resume_model_rejects_malformed_values() {
        for bad in ["", "x", "ab/c"] {
            let mut s = session_with_model("openai/gpt-4o-mini");
            let err = reapply_resume_model(&mut s, &Some(bad.into()))
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
                .unwrap()
                .as_deref(),
            Some("prov/model")
        );
    }
}
