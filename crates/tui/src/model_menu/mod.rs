//! `/config` configuration modal for the TUI.
//!
//! A form overlay (modeled on `menu.rs`) that edits six fields and persists
//! them to `opencoder.json` via [`opencoder_core::Config::save`]:
//! - model id
//! - provider base_url
//! - provider api_key (masked display; editing replaces the value)
//! - reasoning_effort (4-way cycle: off / low / medium / high)
//! - compaction.context_threshold (raw token count)
//! - fps - TUI 渲染帧率（1-30，默认 10）
//!
//! Navigation: `Tab` / `Shift+Tab` (or `\u{2191}`/`\u{2193}`) move focus between options;
//! `\u{2190}`/`\u{2192}` change the value of the focused option;
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
            fps: 10,
            capabilities_browser: false,
            capabilities_computer_use: false,
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
            fps: 10,
            capabilities_browser: false,
            capabilities_computer_use: false,
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
            fps: 10,
            capabilities_browser: false,
            capabilities_computer_use: false,
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
    fn up() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Up, KeyModifiers::empty())
    }
    fn down() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Down, KeyModifiers::empty())
    }
    fn left() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Left, KeyModifiers::empty())
    }
    fn right() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Right, KeyModifiers::empty())
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
            Field::Fps,
            Field::Browser,
            Field::ComputerUse,
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

    // Up/Down always move focus, never change values.
    #[test]
    fn up_down_always_move_focus() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let m = menu_opt.as_mut().unwrap();
        m.focus = Field::Reasoning;
        let before = m.reasoning;
        super::handle_model_key(&mut menu_opt, up());
        let m = menu_opt.as_ref().unwrap();
        assert_eq!(m.focus, Field::ApiKey, "Up on Reasoning moves to ApiKey");
        assert_eq!(m.reasoning, before, "Up must not change reasoning value");

        let m = menu_opt.as_mut().unwrap();
        m.focus = Field::Reasoning;
        super::handle_model_key(&mut menu_opt, down());
        let m = menu_opt.as_ref().unwrap();
        assert_eq!(
            m.focus,
            Field::InterleavedThinking,
            "Down on Reasoning moves to InterleavedThinking"
        );
        assert_eq!(m.reasoning, before, "Down must not change reasoning value");
    }

    // Left/Right always change values, never move focus.
    #[test]
    fn left_right_change_values_on_options() {
        // Reasoning: Left=prev, Right=next
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Reasoning;
        let before = menu_opt.as_ref().unwrap().reasoning;
        super::handle_model_key(&mut menu_opt, right());
        assert_eq!(
            menu_opt.as_ref().unwrap().reasoning,
            before.next(),
            "Right advances reasoning"
        );
        assert_eq!(
            menu_opt.as_ref().unwrap().focus,
            Field::Reasoning,
            "Right must not move focus"
        );

        super::handle_model_key(&mut menu_opt, left());
        assert_eq!(
            menu_opt.as_ref().unwrap().reasoning,
            before,
            "Left returns reasoning to original"
        );

        // Threshold: Right increases, Left decreases
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let t0 = menu_opt.as_ref().unwrap().threshold;
        menu_opt.as_mut().unwrap().focus = Field::Threshold;
        super::handle_model_key(&mut menu_opt, right());
        assert_eq!(
            menu_opt.as_ref().unwrap().threshold,
            t0 + 1000,
            "Right increases threshold by 1k"
        );
        super::handle_model_key(&mut menu_opt, left());
        super::handle_model_key(&mut menu_opt, left());
        assert_eq!(
            menu_opt.as_ref().unwrap().threshold,
            t0.saturating_sub(1000),
            "Left decreases threshold by 1k"
        );
    }

    // Interleave: Left/Right toggle, focus stays put.
    #[test]
    fn left_right_toggle_interleave() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let before = menu_opt.as_ref().unwrap().interleaved_thinking;
        menu_opt.as_mut().unwrap().focus = Field::InterleavedThinking;
        super::handle_model_key(&mut menu_opt, right());
        assert_eq!(
            menu_opt.as_ref().unwrap().interleaved_thinking,
            !before,
            "Right toggles interleave"
        );
        assert_eq!(
            menu_opt.as_ref().unwrap().focus,
            Field::InterleavedThinking,
            "Right must not move focus"
        );
        super::handle_model_key(&mut menu_opt, left());
        assert_eq!(
            menu_opt.as_ref().unwrap().interleaved_thinking,
            before,
            "Left toggles interleave back"
        );
    }

    // Default Config -> fps defaults to 10 (Config::tui_fps clamps None -> 10).
    #[test]
    fn menu_defaults_fps_to_ten() {
        let m = ModelMenu::new(&cfg());
        assert_eq!(m.fps, 10, "default config has no fps -> tui_fps() = 10");
        let patch = m.build_patch();
        assert_eq!(patch.fps, 10);
    }

    // Fps: Left/Right change by ±1, clamped to 1..=30, focus stays put.
    #[test]
    fn left_right_change_fps_by_one() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let f0 = menu_opt.as_ref().unwrap().fps;
        menu_opt.as_mut().unwrap().focus = Field::Fps;
        super::handle_model_key(&mut menu_opt, right());
        assert_eq!(
            menu_opt.as_ref().unwrap().fps,
            f0 + 1,
            "Right increases fps by 1"
        );
        assert_eq!(
            menu_opt.as_ref().unwrap().focus,
            Field::Fps,
            "Right must not move focus"
        );
        super::handle_model_key(&mut menu_opt, left());
        super::handle_model_key(&mut menu_opt, left());
        assert_eq!(
            menu_opt.as_ref().unwrap().fps,
            f0.saturating_sub(1),
            "Left decreases fps by 1"
        );

        // Clamp at the lower bound (1): spam Left from fps=1, must stay 1.
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Fps;
        menu_opt.as_mut().unwrap().fps = 1;
        for _ in 0..5 {
            super::handle_model_key(&mut menu_opt, left());
        }
        assert_eq!(menu_opt.as_ref().unwrap().fps, 1, "fps clamps to 1");

        // Clamp at the upper bound (30): spam Right from fps=30, must stay 30.
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Fps;
        menu_opt.as_mut().unwrap().fps = 30;
        for _ in 0..5 {
            super::handle_model_key(&mut menu_opt, right());
        }
        assert_eq!(menu_opt.as_ref().unwrap().fps, 30, "fps clamps to 30");
    }

    // Fps: typing digits appends to the current value's string form (mirrors
    // the Threshold field's behavior), then clamps to 1..=30.
    #[test]
    fn typing_digits_sets_fps() {
        // From a single-digit starting value, appending one digit yields a
        // two-digit number that stays inside the range.
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Fps;
        menu_opt.as_mut().unwrap().fps = 2;
        super::handle_model_key(&mut menu_opt, key('4'));
        assert_eq!(
            menu_opt.as_ref().unwrap().fps,
            24,
            "from fps=2, typing '4' yields 24"
        );

        // From fps=9, typing '9' yields 99, which clamps down to 30.
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Fps;
        menu_opt.as_mut().unwrap().fps = 9;
        super::handle_model_key(&mut menu_opt, key('9'));
        assert_eq!(
            menu_opt.as_ref().unwrap().fps,
            30,
            "99 clamps to 30"
        );

        // Non-digit characters are ignored on the Fps field.
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        menu_opt.as_mut().unwrap().focus = Field::Fps;
        menu_opt.as_mut().unwrap().fps = 15;
        super::handle_model_key(&mut menu_opt, key('x'));
        assert_eq!(
            menu_opt.as_ref().unwrap().fps,
            15,
            "non-digit chars do not change fps"
        );
    }

    // The fps field is serialized into the patch JSON under the top-level "fps".
    #[test]
    fn patch_serializes_fps() {
        let p = ModelPatch {
            model: "m".into(),
            base_url: "u".into(),
            api_key: None,
            reasoning_effort: None,
            interleaved_thinking: None,
            context_threshold: 1000,
            fps: 25,
            capabilities_browser: true,
            capabilities_computer_use: false,
        };
        let v = p.to_json();
        assert_eq!(v["fps"], serde_json::json!(25));
        assert_eq!(v["capabilities"]["browser"], serde_json::json!(true));
        assert_eq!(v["capabilities"]["computer_use"], serde_json::json!(false));
    }

    // Capabilities init from config: a config with browser+computer_use on
    // must be reflected in ModelMenu::new and round-trip through build_patch.
    #[test]
    fn model_menu_inits_capabilities_from_config() {
        let mut c = cfg();
        c.capabilities.browser = true;
        c.capabilities.computer_use = true;
        let m = ModelMenu::new(&c);
        assert!(m.capabilities_browser, "browser capability read from config");
        assert!(m.capabilities_computer_use, "computer_use capability read from config");
        let patch = m.build_patch();
        assert!(patch.capabilities_browser);
        assert!(patch.capabilities_computer_use);
        // Defaults (browser off) must also round-trip to the patch JSON.
        let m0 = ModelMenu::new(&cfg());
        let v = m0.build_patch().to_json();
        assert_eq!(v["capabilities"]["browser"], serde_json::json!(false));
        assert_eq!(v["capabilities"]["computer_use"], serde_json::json!(false));
    }

    // Space and Left/Right toggle the browser capability checkbox, focus stays.
    #[test]
    fn toggling_browser_capability_checkbox() {
        let mut menu_opt: Option<ModelMenu> = Some(ModelMenu::new(&cfg()));
        let before = menu_opt.as_ref().unwrap().capabilities_browser;
        menu_opt.as_mut().unwrap().focus = Field::Browser;
        super::handle_model_key(&mut menu_opt, key(' '));
        assert_eq!(
            menu_opt.as_ref().unwrap().capabilities_browser,
            !before,
            "Space toggles browser capability"
        );
        assert_eq!(
            menu_opt.as_ref().unwrap().focus,
            Field::Browser,
            "toggling must not move focus"
        );
        super::handle_model_key(&mut menu_opt, right());
        assert_eq!(
            menu_opt.as_ref().unwrap().capabilities_browser,
            before,
            "Right toggles browser back"
        );
        super::handle_model_key(&mut menu_opt, left());
        assert_eq!(
            menu_opt.as_ref().unwrap().capabilities_browser,
            !before,
            "Left toggles browser again"
        );
    }
}
