use super::common::{cfg, right};
use crate::model_menu::{
    config_form::{ConfigForm, Reasoning},
    list::ProviderList,
    provider_form::{self, ProviderField, ProviderForm},
    state::ModelMenu,
};

#[test]
fn response_provider_protocol_survives_form_save_reload_and_edit() {
    let dir = tempfile::tempdir().unwrap();
    let _home = super::common::lock_home(dir.path());
    let mut form = ProviderForm::new_blank(&cfg());
    form.name = "openai".into();
    form.model_id = "gpt-6".into();
    form.base_url = "https://api.openai.com/v1".into();
    form.focus = ProviderField::Protocol;
    let (_, menu) = provider_form::handle_key(form, right());
    let Some(ModelMenu::Form(form)) = menu else {
        panic!("provider form expected")
    };
    let patch = form.build_patch().to_json();
    assert_eq!(patch["providers"]["openai"]["protocol"], "responses");
    opencoder_core::Config::save(dir.path(), &patch).unwrap();
    let config = opencoder_core::Config::load(dir.path()).unwrap();
    let mut list = ProviderList::new(&config);
    assert_eq!(
        list.entries
            .iter()
            .find(|e| e.name == "openai")
            .unwrap()
            .protocol,
        "responses"
    );
    list.selected = list
        .entries
        .iter()
        .position(|e| e.name == "openai")
        .unwrap();
    let (_, menu) = crate::model_menu::list::handle_key(list, super::common::key('e'));
    let Some(ModelMenu::Form(form)) = menu else {
        panic!("edit form expected")
    };
    assert_eq!(form.protocol, "responses");
}

#[test]
fn default_none_minimal_and_custom_effort_are_distinct_and_survive_save() {
    for (input, expected) in [
        (None, None),
        (Some(""), None),
        (Some("none"), Some("none")),
        (Some("minimal"), Some("minimal")),
        (Some("Future-Effort"), Some("Future-Effort")),
    ] {
        let mut config = cfg();
        config.reasoning_effort = input.map(str::to_owned);
        let form = ConfigForm::new(&config);
        assert_eq!(form.build_patch().reasoning_effort.as_deref(), expected);
    }
    assert_eq!(Reasoning::Off.label(), "default");
    assert_eq!(Reasoning::Off.next(), Reasoning::None);
    assert_eq!(Reasoning::None.next(), Reasoning::Minimal);
}
