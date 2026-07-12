//! `/model` configuration modal for the TUI.
//!
//! A form overlay (modeled on `menu.rs`) that edits five fields and persists
//! them to `opencoder.json` via [`opencoder_core::Config::save`]:
//! - model id
//! - provider base_url
//! - provider api_key (masked display; editing replaces the value)
//! - reasoning_effort (4-way cycle: off / low / medium / high)
//! - compaction.context_threshold (raw token count)
//!
//! Navigation: `Tab` / `Shift+Tab` (or `\u{2191}`/`\u{2193}`) move focus;
//! `Enter` on `Save` commits, on `Cancel` aborts; `Esc` cancels. The menu
//! owns no I/O — it returns a [`state::ModelPatch`] and the caller persists it.

pub mod state;
pub mod view;

pub use state::{
    handle_model_key, mask_key, Field, ModelMenu, ModelOutcome, ModelPatch, Reasoning,
};
pub use view::render_model_popup;

#[cfg(test)]
mod tests {
    use super::state::{mask_key, Field, ModelMenu, ModelPatch, Reasoning};
    use crossterm::event::{KeyCode, KeyModifiers};
    use opencoder_core::Config;

    fn cfg() -> Config {
        Config {
            model: "openai/gpt-4o-mini".to_string(),
            provider: opencoder_core::ProviderConfig {
                base_url: "https://api.openai.com/v1".to_string(),
                api_key: Some("sk-abcd1234567".to_string()),
            },
            reasoning_effort: Some("high".to_string()),
            compaction: opencoder_core::CompactionConfig {
                context_threshold: 80_000,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn key(c: char) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    #[test]
    fn patch_carries_edits_and_preserves_untouched_key() {
        let m = ModelMenu::new(&cfg());
        assert_eq!(m.resolve_api_key(), None);
        let patch = m.build_patch();
        assert_eq!(patch.api_key, None);
        assert_eq!(patch.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn editing_api_key_marks_replacement() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        for c in "sk-newkey".chars() {
            if let Some(m) = menu_opt.as_mut() {
                m.focus = Field::ApiKey;
            }
            super::handle_model_key(&mut menu_opt, key(c));
        }
        let m = menu_opt.expect("menu still open");
        assert!(
            m.api_key_edited,
            "api_key field must be marked edited after typing"
        );
        assert_eq!(m.api_key_input, "sk-newkey");
    }

    #[test]
    fn reasoning_cycle_is_circular() {
        let mut r = Reasoning::Off;
        let seq = [
            Reasoning::Low,
            Reasoning::Medium,
            Reasoning::High,
            Reasoning::Off,
        ];
        for expect in seq {
            r = r.next();
            assert_eq!(r, expect);
        }
    }

    #[test]
    fn patch_wraps_env_var_name_in_braces() {
        let p = ModelPatch {
            model: "m".into(),
            base_url: "u".into(),
            api_key: Some("MY_KEY".into()),
            reasoning_effort: None,
            interleaved_thinking: None,
            context_threshold: 1000,
        };
        let v = p.to_json();
        assert_eq!(v["provider"]["api_key"], serde_json::json!("{MY_KEY}"));
    }

    #[test]
    fn patch_omits_api_key_when_untouched() {
        // api_key: None means "leave existing" — must NOT appear in the patch
        // (a `null` would delete the existing key via merge_json).
        let p = ModelPatch {
            model: "m".into(),
            base_url: "u".into(),
            api_key: None,
            reasoning_effort: Some("high".into()),
            interleaved_thinking: None,
            context_threshold: 1000,
        };
        let v = p.to_json();
        let provider_has_key = v
            .get("provider")
            .and_then(|p| p.as_object())
            .is_some_and(|o| o.contains_key("api_key"));
        assert!(
            !provider_has_key,
            "untouched api_key must be absent from patch JSON"
        );
        assert_eq!(v["reasoning_effort"], serde_json::json!("high"));
    }

    #[test]
    fn patch_clears_api_key_when_empty() {
        // api_key: Some("") means explicit clear → null in patch (merge removes).
        let p = ModelPatch {
            model: "m".into(),
            base_url: "u".into(),
            api_key: Some("".into()),
            reasoning_effort: None,
            interleaved_thinking: None,
            context_threshold: 1000,
        };
        let v = p.to_json();
        assert_eq!(v["provider"]["api_key"], serde_json::Value::Null);
    }

    #[test]
    fn mask_hides_short_keys_entirely() {
        assert_eq!(mask_key(""), "(unset)");
        assert_eq!(mask_key("abc"), "****");
        assert_eq!(mask_key("sk-abcd1234567"), "sk****4567");
    }

    fn enter() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
    }

    // Enter on a text field confirms the value and advances focus to the next
    // field — the core "fill → Enter → next" flow requested for the form.
    #[test]
    fn enter_on_text_field_advances_to_next() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        {
            let m = menu_opt.as_ref().unwrap();
            assert_eq!(m.focus, Field::Model, "menu opens focused on Model");
        }
        super::handle_model_key(&mut menu_opt, enter());
        assert_eq!(
            menu_opt.as_ref().unwrap().focus,
            Field::BaseUrl,
            "Enter on Model advances to BaseUrl"
        );
    }

    // Enter on Reasoning must advance WITHOUT toggling the value (Enter is
    // "confirm current selection", not "change it" — cycling stays on Space/←→).
    #[test]
    fn enter_on_reasoning_advances_without_toggling() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let before = menu_opt.as_ref().unwrap().reasoning;
        menu_opt.as_mut().unwrap().focus = Field::Reasoning;
        super::handle_model_key(&mut menu_opt, enter());
        let m = menu_opt.as_ref().unwrap();
        assert_eq!(
            m.focus,
            Field::InterleavedThinking,
            "Enter on Reasoning advances to InterleavedThinking"
        );
        assert_eq!(
            m.reasoning, before,
            "Enter must not change the reasoning value"
        );
    }

    // Repeated Enter walks the whole field order and lands on Save.
    #[test]
    fn enter_chains_through_fields_to_save() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let order = [
            Field::BaseUrl,
            Field::ApiKey,
            Field::Reasoning,
            Field::InterleavedThinking,
            Field::Threshold,
            Field::Save,
        ];
        // starts on Model
        assert_eq!(menu_opt.as_ref().unwrap().focus, Field::Model);
        for expect in order {
            super::handle_model_key(&mut menu_opt, enter());
            assert_eq!(menu_opt.as_ref().unwrap().focus, expect);
        }
    }

    // Enter on Save with valid input commits (returns Save, closes the menu).
    #[test]
    fn enter_on_save_commits_patch() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Save;
        let outcome = super::handle_model_key(&mut menu_opt, enter());
        assert!(
            matches!(outcome, super::ModelOutcome::Save(_)),
            "Enter on Save must commit"
        );
        assert!(menu_opt.is_none(), "menu must close after save");
    }
}
