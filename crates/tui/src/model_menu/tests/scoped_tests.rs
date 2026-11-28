use super::common::{enter, key, lock_home, provider_cfg};
use crate::model_menu::{
    handle_model_key, render_model_popup, ConfigField, ModeConfigForm, ModeField, ModeMenuKind,
    ModeSelection, ModelMenu, ModelOutcome, ProviderList,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{Config, ConfigMode, ModeConfig, ModesConfig};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

fn selection(kind: ModeMenuKind, mode: ConfigMode, config: Config) -> ModeSelection {
    let home = tempfile::tempdir().unwrap();
    let _isolation = lock_home(home.path());
    ModeSelection::new(config, mode, kind, home.path(), None)
}

fn press(slot: &mut Option<ModelMenu>, event: KeyEvent) -> ModelOutcome {
    handle_model_key(slot, event)
}

fn escape() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

fn buffer_text(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..height {
        for x in 0..width {
            if let Some(cell) = buffer.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

#[test]
fn model_scope_renders_only_act_and_plan_with_values() {
    let home = tempfile::tempdir().unwrap();
    let _isolation = lock_home(home.path());
    let config = Config {
        model: "openai/base".into(),
        modes: ModesConfig {
            act: Some(ModeConfig {
                model: Some("openai/act".into()),
                ..Default::default()
            }),
            plan: Some(ModeConfig {
                model: Some("openai/plan".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    // A mode-aware row (`{}`) resolves each scope through the config overlay.
    let menu = ModelMenu::ModeSelect(Box::new(ModeSelection::new(
        config.clone(),
        ConfigMode::Act,
        ModeMenuKind::Model,
        home.path(),
        Some(opencoder_store::SessionModelOverrides::default()),
    )));
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();

    let text = buffer_text(&terminal, 80, 24);
    assert!(text.contains("Choose Act or Plan"), "{text}");
    assert!(text.contains("openai/act"), "{text}");
    assert!(text.contains("openai/plan"), "{text}");
    assert!(!text.contains("Shared"), "Shared must not appear: {text}");

    // A legacy row (`NULL`) keeps single-model semantics: both scopes show
    // the legacy base, exactly like the session resolves them.
    let menu = ModelMenu::ModeSelect(Box::new(selection(
        ModeMenuKind::Model,
        ConfigMode::Act,
        config,
    )));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            render_model_popup(f, Rect::new(0, 0, 80, 24), 23, &menu);
        })
        .unwrap();
    let text = buffer_text(&terminal, 80, 24);
    assert!(text.matches("openai/base").count() >= 2, "{text}");
    assert!(!text.contains("openai/act"), "{text}");
}

#[test]
fn shared_config_cancel_and_escape_return_to_scope_selection() {
    let mut slot = Some(ModelMenu::ModeSelect(Box::new(selection(
        ModeMenuKind::Config,
        ConfigMode::Shared,
        Config::default(),
    ))));
    assert!(matches!(press(&mut slot, enter()), ModelOutcome::Idle));
    let Some(ModelMenu::Config(form)) = &mut slot else {
        panic!("Shared config form should open")
    };
    form.focus = ConfigField::Cancel;

    assert!(matches!(press(&mut slot, enter()), ModelOutcome::Idle));
    assert!(matches!(slot, Some(ModelMenu::ModeSelect(_))));

    assert!(matches!(press(&mut slot, enter()), ModelOutcome::Idle));
    assert!(matches!(press(&mut slot, escape()), ModelOutcome::Idle));
    assert!(matches!(slot, Some(ModelMenu::ModeSelect(_))));
}

#[test]
fn act_model_selection_emits_only_the_act_session_patch() {
    let mut slot = Some(ModelMenu::ModeSelect(Box::new(selection(
        ModeMenuKind::Model,
        ConfigMode::Act,
        provider_cfg(),
    ))));
    press(&mut slot, enter());
    assert!(matches!(slot, Some(ModelMenu::List(_))));
    press(&mut slot, enter());

    let outcome = press(&mut slot, key('n'));
    assert!(slot.is_none());
    let ModelOutcome::SaveSessionOnly(patch) = outcome else {
        panic!("session-only Act selection should be returned")
    };
    assert_eq!(
        patch,
        serde_json::json!({
            "modes": { "act": { "model": "deepseek/deepseek-chat" } }
        })
    );
}

#[test]
fn scoped_provider_form_returns_to_list_then_scope() {
    let mut slot = Some(ModelMenu::List(ProviderList::for_mode(
        selection(ModeMenuKind::Model, ConfigMode::Plan, provider_cfg()),
        ConfigMode::Plan,
    )));
    press(&mut slot, key('e'));
    assert!(matches!(slot, Some(ModelMenu::Form(_))));

    assert!(matches!(press(&mut slot, escape()), ModelOutcome::Idle));
    let Some(ModelMenu::List(list)) = &slot else {
        panic!("provider form should return to its scoped list")
    };
    assert_eq!(list.mode, ConfigMode::Plan);

    assert!(matches!(press(&mut slot, escape()), ModelOutcome::Idle));
    assert!(matches!(slot, Some(ModelMenu::ModeSelect(_))));
}

#[test]
fn act_config_save_uses_null_to_restore_shared_inheritance() {
    let config = Config {
        modes: ModesConfig {
            shared: ModeConfig {
                reasoning_effort: Some("high".into()),
                max_tokens: Some(2_000),
                context_limit: Some(20_000),
                ..Default::default()
            },
            act: Some(ModeConfig {
                model: None,
                reasoning_effort: Some("low".into()),
                max_tokens: Some(4_000),
                context_limit: Some(30_000),
                interleaved_thinking: Some(true),
                cache_salt: Some(false),
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let source = selection(ModeMenuKind::Config, ConfigMode::Act, config);
    let mut form = ModeConfigForm::new(source, ConfigMode::Act);
    form.reasoning_inherit = true;
    form.max_tokens_input.clear();
    form.context_limit_input.clear();
    form.interleaved_thinking = None;
    form.cache_salt = None;
    form.focus = ModeField::Save;
    let mut slot = Some(ModelMenu::ModeConfig(form));

    let outcome = press(&mut slot, enter());
    let ModelOutcome::Save(patch) = outcome else {
        panic!("valid Act config should produce a save patch")
    };
    assert_eq!(
        patch,
        serde_json::json!({
            "modes": { "act": {
                "reasoning_effort": null,
                "interleaved_thinking": null,
                "max_tokens": null,
                "context_limit": null,
                "cache_salt": null
            } }
        })
    );

    let source = selection(ModeMenuKind::Config, ConfigMode::Plan, Config::default());
    let mut form = ModeConfigForm::new(source, ConfigMode::Plan);
    form.focus = ModeField::Cancel;
    let mut slot = Some(ModelMenu::ModeConfig(form));
    assert!(matches!(press(&mut slot, enter()), ModelOutcome::Idle));
    assert!(matches!(slot, Some(ModelMenu::ModeSelect(_))));
}
