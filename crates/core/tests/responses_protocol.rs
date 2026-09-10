use opencoder_core::{Config, ProviderConfig, ProviderProtocol};
use serde_json::json;

#[test]
fn protocol_round_trips_and_routes_legacy_and_named_providers() {
    let old: ProviderConfig =
        serde_json::from_value(json!({"base_url":"http://localhost"})).unwrap();
    assert_eq!(old.protocol, "chat_completions");
    assert_eq!(ProviderConfig::default().protocol, "chat_completions");
    let config = Config::default().merged_with(&json!({"model":"gateway/gpt-6-astra","provider":{
        "base_url":"http://localhost", "api_key":"fixture-key", "protocol":"responses"
    }}));
    let endpoint = config.resolve_endpoint().unwrap();
    assert_eq!(endpoint.protocol, ProviderProtocol::Responses);
    assert_eq!(endpoint.provider, "gateway");
    let named=config.merged_with(&json!({"providers":{"gateway":{"protocol":"chat_completions","base_url":"http://127.0.0.1"}}}));
    assert_eq!(
        named.resolve_endpoint().unwrap().protocol,
        ProviderProtocol::ChatCompletions
    );
    assert_eq!(
        serde_json::to_value(named).unwrap()["providers"]["gateway"]["protocol"],
        "chat_completions"
    );
    let invalid = config.merged_with(&json!({"provider":{"protocol":"typo"}}));
    assert!(invalid
        .resolve_endpoint()
        .unwrap_err()
        .to_string()
        .contains("protocol"));
}

#[test]
fn protocol_save_load_merge_delete_and_invalid_patch_are_explicit() {
    let home = tempfile::tempdir().unwrap();
    let _guard = opencoder_core::scoped_config_home(home.path().to_path_buf());
    let dir = tempfile::tempdir().unwrap();
    let path = Config::save(
        dir.path(),
        &json!({"model":"gateway/gpt-5","providers":{"gateway":{
            "protocol":"responses","base_url":"http://localhost","api_key":"fixture-key"
        }}}),
    )
    .unwrap();
    let loaded = Config::load(dir.path()).unwrap();
    assert_eq!(
        loaded.resolve_endpoint().unwrap().protocol,
        ProviderProtocol::Responses
    );
    Config::save(
        dir.path(),
        &json!({"providers":{"gateway":{"model":"gpt-6-astra"}}}),
    )
    .unwrap();
    assert_eq!(
        Config::load(dir.path()).unwrap().providers["gateway"].protocol,
        "responses"
    );
    let before = std::fs::read(&path).unwrap();
    for invalid in [json!("response"), json!(42), json!(true)] {
        assert!(Config::save(
            dir.path(),
            &json!({"providers":{"gateway":{"protocol":invalid}}})
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    Config::save(
        dir.path(),
        &json!({"providers":{"gateway":{"protocol":null}}}),
    )
    .unwrap();
    assert_eq!(
        Config::load(dir.path()).unwrap().providers["gateway"].protocol,
        "chat_completions"
    );
    std::fs::write(&path, r#"{"provider":{"protocol":false}}"#).unwrap();
    assert!(Config::load(dir.path())
        .unwrap_err()
        .to_string()
        .contains("protocol"));
}
