//! Shared test helpers and cross-cutting tests (mask_key, Reasoning cycle).

use crate::model_menu::config_form::Reasoning;
use crate::model_menu::state::mask_key;
use crossterm::event::{KeyCode, KeyModifiers};
use opencoder_core::Config;

pub(crate) fn cfg() -> Config {
    Config {
        model: "openai/gpt-4o-mini".to_string(),
        provider: opencoder_core::ProviderConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some("sk-abcd1234567".to_string()),
            model: None,
            headers: Vec::new(),
        },
        reasoning_effort: Some("high".to_string()),
        compaction: opencoder_core::CompactionConfig {
            context_threshold: 80_000,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A config with one custom provider named "deepseek" that is the active model.
pub(crate) fn provider_cfg() -> Config {
    let mut c = cfg();
    c.model = "deepseek/deepseek-chat".to_string();
    c.providers.insert(
        "deepseek".to_string(),
        opencoder_core::ProviderConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some("dk-secret-key".to_string()),
            model: Some("deepseek-chat".to_string()),
            headers: vec![opencoder_core::HttpHeader {
                name: "X-Region".into(),
                value: "eu".into(),
            }],
        },
    );
    c
}

pub(crate) fn key(c: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}
pub(crate) fn enter() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
}
pub(crate) fn left() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Left, KeyModifiers::empty())
}
pub(crate) fn right() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Right, KeyModifiers::empty())
}
pub(crate) fn esc() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())
}

// ── mask_key ──────────────────────────────────────────────────────────────

#[test]
fn mask_hides_short_keys_entirely() {
    assert_eq!(mask_key(""), "(unset)");
    assert_eq!(mask_key("abc"), "****");
    assert_eq!(mask_key("sk-abcd1234567"), "sk****4567");
}

// ── Reasoning ─────────────────────────────────────────────────────────────

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
