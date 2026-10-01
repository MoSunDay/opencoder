use opencoder_core::Config;
use serde_json::json;

#[test]
fn server_connection_defaults_off_and_merges_fields_independently() {
    let config = Config::default();
    assert!(!config.opencoder_server.enabled);
    let enabled = config
        .merged_with(&json!({"opencoder_server":{"enabled":true,"url":"http://localhost:8080"}}));
    assert!(enabled.opencoder_server.enabled);
    assert_eq!(enabled.opencoder_server.url, "http://localhost:8080");
    let disabled = enabled.merged_with(&json!({"opencoder_server":{"enabled":false}}));
    assert!(!disabled.opencoder_server.enabled);
    assert_eq!(disabled.opencoder_server.url, "http://localhost:8080");
    let encoded = serde_json::to_value(disabled).unwrap();
    assert!(encoded["opencoder_server"].get("token").is_none());
}

#[test]
fn project_server_connection_round_trips_without_a_token_field() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let _scope = opencoder_core::config::scoped_config_home(home);
    std::fs::write(
        workdir.join("opencoder.json"),
        json!({"opencoder_server":{"enabled":true,"url":"http://localhost:8080"}}).to_string(),
    )
    .unwrap();
    let loaded = Config::load(&workdir).unwrap();
    assert!(loaded.opencoder_server.enabled);
    assert_eq!(loaded.opencoder_server.url, "http://localhost:8080");
    Config::save(&workdir, &serde_json::to_value(&loaded).unwrap()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(workdir.join("opencoder.json")).unwrap()).unwrap();
    assert_eq!(saved["opencoder_server"]["url"], "http://localhost:8080");
    assert!(saved["opencoder_server"].get("token").is_none());
}
