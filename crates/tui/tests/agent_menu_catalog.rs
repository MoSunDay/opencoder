//! TUI reads Server registration; local cards never become remote choices.
use opencoder_core::Config;
use opencoder_tui::{
    agent_menu::{AgentCard, AgentMenu},
    remote,
};

#[tokio::test]
async fn disabled_connection_offers_self_even_with_local_cards() {
    let root = tempfile::tempdir().unwrap();
    let card = root.path().join("local-only");
    std::fs::create_dir_all(&card).unwrap();
    std::fs::write(card.join("meta.json"), r#"{"name":"local-only"}"#).unwrap();
    let mut config = Config::default();
    config.agent.agents_dir = Some(root.path().to_owned());
    let cards = remote::catalog(&config).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].name, "self");
}

#[test]
fn capability_picker_filters_kind_and_summary_without_collapsing_targets() {
    let mut menu = AgentMenu::new(vec![
        AgentCard {
            name: "self".into(),
            description: "本地执行".into(),
        },
        AgentCard {
            name: "deploy".into(),
            description: "operator · codex · 发布".into(),
        },
        AgentCard {
            name: "review".into(),
            description: "agent · codex · 审查".into(),
        },
    ]);
    for ch in "operator".chars() {
        menu.on_char(ch);
    }
    assert_eq!(menu.visible_count(), 1);
    assert_eq!(menu.selected_agent().unwrap().name, "deploy");
    for _ in 0..8 {
        menu.on_backspace();
    }
    for ch in "codex".chars() {
        menu.on_char(ch);
    }
    assert_eq!(menu.visible_count(), 2);
}

#[test]
fn server_url_requires_http_and_never_embeds_credentials() {
    assert_eq!(
        remote::client::normalize_url("  http://localhost:8080/  ").unwrap(),
        "http://localhost:8080"
    );
    for url in [
        "",
        "localhost:8080",
        "file:///tmp/server",
        "https://user:secret@example.test",
        "https://example.test?token=secret",
        "https://example.test#token",
    ] {
        assert!(remote::client::normalize_url(url).is_err(), "{url}");
    }
}

#[tokio::test]
async fn self_does_not_require_a_server_or_provider_and_unknown_remote_fails() {
    let mut config = Config::default();
    config.opencoder_server.enabled = true;
    config.opencoder_server.url = "invalid".into();
    assert!(matches!(
        remote::select(&config, "self").await.unwrap(),
        opencoder_tui::task::TaskPick::New
    ));
    assert!(remote::select(&config, "other").await.is_err());
}
