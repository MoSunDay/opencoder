use super::*;

#[test]
fn flash_visible_within_window() {
    assert!(flash_visible(10, 11, 5));
    assert!(flash_visible(10, 14, 5));
}

#[test]
fn flash_visible_expired() {
    assert!(!flash_visible(10, 15, 5));
    assert!(!flash_visible(10, 99, 5));
}

#[test]
fn flash_visible_handles_wraparound() {
    // start near u32::MAX; `now` wraps past 0. Ages 0..4 -> visible, 5 -> expired.
    assert!(flash_visible(u32::MAX, u32::MAX, 5));
    assert!(flash_visible(u32::MAX, 0, 5));
    assert!(flash_visible(u32::MAX, 3, 5));
    assert!(!flash_visible(u32::MAX, 4, 5));
    assert!(!flash_visible(u32::MAX, 99, 5));
}

/// `start_turn` must report failure when the worker command channel has no
/// consumer — the exact signature of a dead worker task (panic or unexpected
/// exit). The main loop relies on this `false` to surface a marker and exit
/// instead of silently queuing into a void and spinning the spinner forever.
#[tokio::test]
async fn start_turn_reports_false_when_worker_is_dead() {
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::worker::UiCmd;

    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCmd>(8);
    drop(cmd_rx); // worker gone — channel closed
    let mut cancel = CancellationToken::new();
    let ok = crate::app_helpers::start_turn(
        &cmd_tx,
        &mut cancel,
        UiCmd::Prompt("hi".into(), Vec::new()),
    )
    .await;
    assert!(
        !ok,
        "start_turn must return false when the worker channel is closed"
    );
}

/// `worker_dead` surfaces a visible marker so the user understands the engine
/// stopped (rather than an unexplained freeze).
#[test]
fn worker_dead_pushes_a_marker() {
    let mut chat = crate::chat::ChatView::default();
    crate::app::worker_dead(&mut chat);
    let text = crate::chat::block_text(&chat);
    assert!(
        text.contains("worker stopped"),
        "expected a worker-stopped marker; got: {text}"
    );
}

#[test]
fn double_esc_while_running_cancels() {
    // Two Esc presses within ESC_CANCEL_WINDOW_MS while running should produce
    // KeyAction::Cancel (hard-abort). The first press records the timestamp;
    // the second, falling inside the window, returns Cancel.
    let history: Vec<String> = vec![];
    let mut input = String::from("draft");
    let mut idx = 5;
    let mut hist_idx = None;
    let mut scroll = 0u32;
    let mut follow = true;
    let mut last_esc: Option<Instant> = None;
    let mut skill_menu: Option<SkillMenu> = None;
    let mut undo_state = crate::undo::init("", 0);
    let mut queue_scroll: u32 = 0;
    let esc = key(KeyCode::Esc, KeyModifiers::NONE);

    let first = handle_key(
        esc,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut idx,
        &history,
        &mut hist_idx,
        true,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(
        matches!(first, KeyAction::None),
        "first esc is a soft clear"
    );
    assert!(last_esc.is_some(), "first esc records the timestamp");

    let second = handle_key(
        esc,
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut idx,
        &history,
        &mut hist_idx,
        true,
        false,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        false,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut None,
    );
    assert!(
        matches!(second, KeyAction::Cancel),
        "double esc within the window must hard-abort"
    );
}

#[test]
fn startup_endpoint_resolves_by_model_prefix_not_legacy_field() {
    use opencoder_core::{Config, ProviderConfig};
    use std::collections::HashMap;
    let mut providers = HashMap::new();
    providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            protocol: "chat_completions".into(),
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some("dk-key".to_string()),
            model: None,
            headers: Vec::new(),
        },
    );
    let cfg = Config {
        model: "deepseek/deepseek-chat".to_string(),
        // Legacy single-provider field — the value the OLD startup bug picked.
        // Distinct from providers["deepseek"] so a revert to the raw field is
        // caught (it would return the openai url + oai-key instead).
        provider: ProviderConfig {
            protocol: "chat_completions".into(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some("oai-key".to_string()),
            model: None,
            headers: Vec::new(),
        },
        providers,
        ..Default::default()
    };
    let ep = crate::app_helpers::startup_endpoint(&cfg).unwrap();
    assert_eq!(ep.base_url, "https://api.deepseek.com/v1");
    assert_eq!(ep.api_key, "dk-key");
}

#[test]
fn startup_endpoint_falls_back_to_legacy_when_prefix_absent() {
    use opencoder_core::{Config, ProviderConfig};
    use std::collections::HashMap;
    // Model prefix "unknown-svc" is not in providers -> fall back to the
    // legacy top-level provider field (boundary case for the startup seam).
    let cfg = Config {
        model: "unknown-svc/model-x".to_string(),
        provider: ProviderConfig {
            protocol: "chat_completions".into(),
            base_url: "https://legacy.example.com/v1".to_string(),
            api_key: Some("legacy-key".to_string()),
            model: None,
            headers: Vec::new(),
        },
        providers: HashMap::new(),
        ..Default::default()
    };
    let ep = crate::app_helpers::startup_endpoint(&cfg).unwrap();
    assert_eq!(ep.base_url, "https://legacy.example.com/v1");
    assert_eq!(ep.api_key, "legacy-key");
}

#[test]
fn size_changed_detects_dimension_change() {
    use crate::app_helpers::size_changed;
    assert!(
        size_changed(Some((80, 24)), (80, 25)),
        "height change must count"
    );
    assert!(
        size_changed(Some((80, 24)), (81, 24)),
        "width change must count"
    );
}

#[test]
fn size_changed_false_when_unchanged() {
    use crate::app_helpers::size_changed;
    assert!(!size_changed(Some((80, 24)), (80, 24)));
}

#[test]
fn size_changed_true_when_no_prior_reading() {
    use crate::app_helpers::size_changed;
    assert!(size_changed(None, (80, 24)));
}

#[test]
fn size_changed_false_for_zero_dimensions() {
    // 0x0 is a transient glitch on minimize/detach; it should not be treated
    // as a real resize target so we avoid spurious autoresize + re-render.
    use crate::app_helpers::size_changed;
    assert!(
        !size_changed(Some((80, 24)), (0, 24)),
        "zero width must not count as a resize"
    );
    assert!(
        !size_changed(Some((80, 24)), (80, 0)),
        "zero height must not count as a resize"
    );
    assert!(
        !size_changed(None, (0, 0)),
        "zero dims from first frame must not count"
    );
}
