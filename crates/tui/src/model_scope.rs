//! Mode-effective configuration shared by TUI display and model/config menus.
//!
//! Session-only models stay in SessionState. ChatView carries a display copy
//! so title/context rendering can resolve the same view without writing those
//! selections back into configuration files.

use opencoder_core::{Config, ConfigMode};
use opencoder_store::SessionModelOverrides;

use crate::chat::ChatView;

pub fn mode_for_agent(agent_name: &str) -> ConfigMode {
    match agent_name {
        "act" => ConfigMode::Act,
        "plan" => ConfigMode::Plan,
        _ => ConfigMode::Shared,
    }
}

pub fn mode_name(mode: ConfigMode) -> &'static str {
    match mode {
        ConfigMode::Shared => "Shared",
        ConfigMode::Act => "Act",
        ConfigMode::Plan => "Plan",
    }
}

fn with_session_base(config: &Config, shared_model: &str) -> Config {
    let mut base = config.clone();
    if !shared_model.is_empty() {
        base.model = shared_model.to_string();
    }
    base
}

/// Durable Shared selection carried by a session, independent of mode overlays.
pub fn shared_model(config: &Config, overrides: Option<&SessionModelOverrides>) -> String {
    overrides
        .and_then(|items| items.shared.clone())
        .unwrap_or_else(|| config.effective_for_mode(ConfigMode::Shared).model)
}

/// Resolve a mode using the same legacy-row and session-overlay rules as
/// SessionState, while custom agents always select Shared.
pub fn effective_for_mode(
    config: &Config,
    shared_model: &str,
    overrides: Option<&SessionModelOverrides>,
    mode: ConfigMode,
) -> Config {
    let base = with_session_base(config, shared_model);
    let mut effective = match (mode, overrides) {
        (ConfigMode::Act | ConfigMode::Plan, None) => base,
        _ => base.effective_for_mode(mode),
    };
    if let Some(model) = overrides.and_then(|items| items.get(mode)) {
        effective.model = model.to_string();
    }
    effective
}

pub fn effective_for_chat(config: &Config, chat: &ChatView) -> Config {
    effective_for_mode(
        config,
        &chat.shared_model,
        chat.model_overrides.as_ref(),
        mode_for_agent(&chat.agent),
    )
}

/// A display configuration carrying every session-only model but no persisted
/// changes. ModeSelection applies the requested mode from this base.
pub fn menu_config(config: &Config, chat: &ChatView) -> Config {
    with_session_base(config, &chat.shared_model)
}

pub fn context_limit(config: &Config, chat: &ChatView, fallback: u64) -> u64 {
    if chat.shared_model.is_empty() && chat.model_overrides.is_none() {
        fallback
    } else {
        effective_for_chat(config, chat).context_limit()
    }
}

/// `/model` only ever selects Act or Plan: a top-level `model` key (the
/// Shared default, written by onboarding or `/config`) is deliberately not a
/// session selection.
pub fn selection_from_patch(patch: &serde_json::Value) -> Option<(ConfigMode, String)> {
    let modes = patch.get("modes")?.as_object()?;
    for (key, mode) in [("act", ConfigMode::Act), ("plan", ConfigMode::Plan)] {
        if let Some(model) = modes
            .get(key)
            .and_then(|value| value.get("model"))
            .and_then(|value| value.as_str())
        {
            return Some((mode, model.to_string()));
        }
    }
    None
}

/// Outer ChatClient rebuilding is needed for provider/endpoint changes, not
/// for selecting a model that the configured client already routes by prefix.
pub fn needs_client_rebuild(patch: &serde_json::Value) -> bool {
    patch.get("providers").is_some() || patch.get("network").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::{ModeConfig, ModesConfig};

    #[test]
    fn effective_mode_uses_override_and_custom_agents_use_shared() {
        let config = Config {
            model: "p/base".into(),
            context_limit: Some(10_000),
            modes: ModesConfig {
                shared: ModeConfig {
                    model: Some("p/shared".into()),
                    context_limit: Some(9_000),
                    ..Default::default()
                },
                act: Some(ModeConfig {
                    model: Some("p/act".into()),
                    context_limit: Some(8_000),
                    ..Default::default()
                }),
                plan: None,
            },
            ..Default::default()
        };
        let overrides = SessionModelOverrides {
            shared: None,
            act: Some("p/temp".into()),
            plan: None,
        };
        assert_eq!(
            effective_for_mode(&config, "p/base", Some(&overrides), ConfigMode::Act).model,
            "p/temp"
        );
        assert_eq!(mode_for_agent("custom"), ConfigMode::Shared);
        assert_eq!(
            effective_for_mode(&config, "p/base", Some(&overrides), ConfigMode::Shared)
                .context_limit(),
            9_000
        );
    }

    #[test]
    fn shared_display_inherits_overlay_unless_session_pins_it() {
        let config = Config {
            model: "p/base".into(),
            modes: ModesConfig {
                shared: ModeConfig {
                    model: Some("p/config-shared".into()),
                    max_tokens: Some(4_096),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let empty = SessionModelOverrides::default();
        assert_eq!(shared_model(&config, Some(&empty)), "p/config-shared");
        assert_eq!(
            effective_for_mode(&config, "p/config-shared", Some(&empty), ConfigMode::Shared)
                .max_tokens,
            Some(4_096)
        );

        let pinned = SessionModelOverrides {
            shared: Some("p/session-shared".into()),
            ..Default::default()
        };
        assert_eq!(shared_model(&config, Some(&pinned)), "p/session-shared");
        assert_eq!(
            effective_for_mode(
                &config,
                "p/session-shared",
                Some(&pinned),
                ConfigMode::Shared
            )
            .model,
            "p/session-shared"
        );
    }

    #[test]
    fn patch_selection_reads_only_act_and_plan() {
        assert_eq!(
            selection_from_patch(&serde_json::json!({"model":"p/shared"})),
            None,
            "the Shared default is never a `/model` session selection"
        );
        assert_eq!(
            selection_from_patch(&serde_json::json!({"modes":{"act":{"model":"p/a"}}})),
            Some((ConfigMode::Act, "p/a".into()))
        );
        assert_eq!(
            selection_from_patch(&serde_json::json!({"modes":{"plan":{"model":"p/p"}}})),
            Some((ConfigMode::Plan, "p/p".into()))
        );
        assert_eq!(
            selection_from_patch(&serde_json::json!({"modes":{"shared":{"model":"p/s"}}})),
            None
        );
        assert_eq!(
            selection_from_patch(&serde_json::json!({"providers":{}})),
            None
        );
        assert!(!needs_client_rebuild(&serde_json::json!({"model":"p/a"})));
    }
}
