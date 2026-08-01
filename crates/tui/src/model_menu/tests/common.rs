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
pub(crate) fn backspace() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty())
}
/// Ctrl+<c> chord (the char form terminals report for Ctrl+L / Ctrl+U).
pub(crate) fn ctrl(c: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

// ── HOME isolation ─────────────────────────────────────────────────────────
//
// `Config::save_target` walks `config_candidates(working_dir)`, which appends
// the global files `~/.opencoder/config.json` / `~/.opencoder/opencoder.json`
// and the XDG `~/.config/opencoder/config.json`. A test that passes a tempdir
// as `working_dir` but leaves the real `HOME`/`XDG_CONFIG_HOME` in place causes
// `save_target` to fall through the (non-existent) project-local candidates to
// the *real* global config and overwrite the user's `~/.opencoder/config.json`
// (observed: `model` clobbered to a test value). `lock_home` repoints both env
// vars at the tempdir for the lifetime of the returned guard, so every global
// candidate resolves *inside* the tempdir and the real user config is untouched.
//
// The guard holds the process-wide `HOME_TEST_LOCK` — the very same mutex the
// `app_loop` / `sys_tokens_*` tests use — so HOME mutators and HOME readers are
// serialized across the whole `opencoder-tui` test binary. Two independent
// mutexes would NOT interlock: a concurrent `sys_tokens_counts_system_prompt`
// reader (which indirectly reads `home_dir()`) could observe an empty tempdir
// HOME and flake its `base > 0` assertion (the classic 0-vs-406 race).
pub(crate) struct HomeGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

/// Point `HOME` + `XDG_CONFIG_HOME` at `home` for the lifetime of the returned
/// guard, restoring the prior values (and releasing the shared lock) on drop.
pub(crate) fn lock_home(home: &std::path::Path) -> HomeGuard {
    let _lock = crate::app::app_loop::tests::HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prev_home = std::env::var_os("HOME");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("HOME", home);
    std::env::set_var("XDG_CONFIG_HOME", home);
    HomeGuard {
        prev_home,
        prev_xdg,
        _lock,
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match self.prev_xdg.take() {
            Some(h) => std::env::set_var("XDG_CONFIG_HOME", h),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
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
        Reasoning::XHigh,
        Reasoning::Max,
        Reasoning::Off,
    ];
    for expect in seq {
        r = r.next();
        assert_eq!(r, expect);
    }
}

#[test]
fn reasoning_new_levels_round_trip() {
    // from_config parses the extended effort values xhigh / max.
    assert_eq!(Reasoning::from_config(Some("xhigh")), Reasoning::XHigh);
    assert_eq!(Reasoning::from_config(Some("max")), Reasoning::Max);
    // case-insensitive + trimmed, like the other levels.
    assert_eq!(Reasoning::from_config(Some("  XHigh ")), Reasoning::XHigh);
    // unknown strings still fall back to Off (field omitted).
    assert_eq!(Reasoning::from_config(Some("ultra")), Reasoning::Off);

    // to_option emits the literal provider tokens.
    assert_eq!(Reasoning::XHigh.to_option().as_deref(), Some("xhigh"));
    assert_eq!(Reasoning::Max.to_option().as_deref(), Some("max"));
    assert_eq!(Reasoning::Off.to_option(), None);

    // Full parse -> serialize round-trip for the whole scale.
    for variant in [
        Reasoning::Off,
        Reasoning::Low,
        Reasoning::Medium,
        Reasoning::High,
        Reasoning::XHigh,
        Reasoning::Max,
    ] {
        let s = variant.to_option();
        assert_eq!(Reasoning::from_config(s.as_deref()), variant);
    }

    // prev inverts next across the new variants.
    assert_eq!(Reasoning::XHigh.prev(), Reasoning::High);
    assert_eq!(Reasoning::Max.prev(), Reasoning::XHigh);
    assert_eq!(Reasoning::High.next(), Reasoning::XHigh);
    assert_eq!(Reasoning::XHigh.next(), Reasoning::Max);
}
