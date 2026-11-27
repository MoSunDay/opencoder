use super::common::{enter, key, lock_home, provider_cfg};
use crate::model_menu::{
    handle_model_key, ConfigField, ModeConfigForm, ModeField, ModeMenuKind, ModeSelection,
    ModelMenu, ModelOutcome, ProviderList,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{Config, ConfigMode, ModeConfig, ModesConfig};

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
