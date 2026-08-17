//! First-run model setup inside the terminal UI.
//!
//! The wizard reuses the `/model` provider form, but owns a small startup loop
//! so no Store, Session, or worker exists until a locally usable client can be
//! built. It never probes the provider over the network.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use crossterm::event::{Event, KeyEventKind};
use opencoder_core::Config;
use opencoder_llm::ChatClient;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::input::spawn_input_pump;
use crate::model_menu::{handle_model_key, ModelMenu, ModelOutcome, ProviderForm};
use crate::render::Term;

pub(crate) enum OnboardingOutcome {
    Ready {
        config: Box<Config>,
        client: ChatClient,
    },
    Exit,
}

/// Why startup client construction failed, split by whether the onboarding
/// wizard can actually fix it.
pub(crate) enum StartupFailure {
    /// endpoint/凭据解析失败——向导能修（缺 provider/api_key/base_url 等）。
    Credentials(anyhow::Error),
    /// endpoint 已解析但客户端构建失败（proxy env 非法、header 值非法、
    /// base_url 非 http(s) 等）——向导修不了，进应用后按 turn 报错。
    Unbuildable(anyhow::Error),
}

impl StartupFailure {
    /// Collapse back to a plain error for wizard form display: inside the
    /// wizard both classes surface the same way (Save keeps failing).
    fn into_error(self) -> anyhow::Error {
        match self {
            StartupFailure::Credentials(e) | StartupFailure::Unbuildable(e) => e,
        }
    }
}

/// Strict local readiness check. Endpoint resolution catches missing secrets
/// ([`StartupFailure::Credentials`]); URL/header parsing catches values that
/// reqwest would otherwise reject only after the user's first task, and client
/// construction validates proxy settings — both map to
/// [`StartupFailure::Unbuildable`] since the wizard cannot fix them.
pub(crate) fn build_ready_client(config: &Config) -> Result<ChatClient, StartupFailure> {
    let ep = crate::app_helpers::startup_endpoint(config)
        .context("model credentials")
        .map_err(StartupFailure::Credentials)?;
    let url = reqwest::Url::parse(&ep.base_url)
        .context("invalid base_url")
        .map_err(StartupFailure::Unbuildable)?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() || !url.has_host() {
        return Err(StartupFailure::Unbuildable(anyhow!(
            "base_url must be an absolute http(s) URL"
        )));
    }
    for (name, value) in &ep.headers {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid header name `{name}`"))
            .map_err(StartupFailure::Unbuildable)?;
        reqwest::header::HeaderValue::from_str(value)
            .with_context(|| format!("invalid value for header `{name}`"))
            .map_err(StartupFailure::Unbuildable)?;
    }
    ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )
    .map_err(StartupFailure::Unbuildable)
}

/// Fallback `ChatStream` used when the model client is unbuildable but the
/// session should still start (e.g. an invalid proxy env that the onboarding
/// wizard cannot fix). Every turn submission fails immediately with the
/// recorded reason, surfacing the root cause as a turn-level error instead of
/// trapping the user in the wizard.
pub(crate) struct UnbuildableClient {
    pub reason: String,
}

impl opencoder_llm::ChatStream for UnbuildableClient {
    fn chat_stream(
        &self,
        _req: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        anyhow::bail!("model client unavailable: {}", self.reason)
    }

    fn backend(&self) -> &'static str {
        "unavailable"
    }
}

/// Drive first-run setup until a valid global model is saved or the user exits.
pub(crate) async fn run(
    terminal: &mut Term,
    workdir: &Path,
    cli_model: Option<&str>,
    config: Config,
    startup_error: anyhow::Error,
) -> Result<OnboardingOutcome> {
    let mut form = ProviderForm::new_onboarding(&config);
    form.error = Some(format!("setup required: {startup_error:#}"));

    let heartbeat = crate::supervisor::Heartbeat::new();
    let active = Arc::new(AtomicBool::new(true));
    crate::supervisor::spawn(heartbeat.clone(), Arc::clone(&active));
    let (mut input_rx, _input_handle) = spawn_input_pump(heartbeat);

    let result = async {
        loop {
            terminal.draw(|frame| render(frame, &form))?;
            let Some(event) = input_rx.recv().await else {
                return Ok(OnboardingOutcome::Exit);
            };
            match event {
                Event::Paste(text) => form.paste_into(&text),
                Event::Resize(_, _) => continue,
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let before = form.clone();
                    let mut slot = Some(ModelMenu::Form(form));
                    match handle_model_key(&mut slot, key) {
                        ModelOutcome::Idle => {
                            form = take_form(slot).unwrap_or(before);
                        }
                        ModelOutcome::Cancel | ModelOutcome::Quit => {
                            return Ok(OnboardingOutcome::Exit);
                        }
                        ModelOutcome::Save(patch) => {
                            match finalize_config(workdir, cli_model, &config, &patch) {
                                Ok((reloaded, client)) => {
                                    return Ok(OnboardingOutcome::Ready {
                                        config: Box::new(reloaded),
                                        client,
                                    });
                                }
                                Err(error) => {
                                    form = before;
                                    form.error = Some(format!("setup not complete: {error:#}"));
                                }
                            }
                        }
                        ModelOutcome::SaveSessionOnly(_) => {
                            form = before;
                            form.error = Some("first setup must be saved globally".into());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    .await;

    active.store(false, Ordering::Relaxed);
    drop(input_rx);
    result
}

fn finalize_config(
    workdir: &Path,
    cli_model: Option<&str>,
    current: &Config,
    patch: &serde_json::Value,
) -> Result<(Config, ChatClient)> {
    let candidate = current.merged_with(patch);
    build_ready_client(&candidate)
        .map_err(StartupFailure::into_error)
        .context("candidate model is unusable")?;
    Config::save_global(patch).context("save global config")?;
    let mut reloaded = Config::load(workdir).context("reload effective config")?;
    if let Some(model) = cli_model {
        reloaded.model = model.to_string();
    }
    let client = build_ready_client(&reloaded)
        .map_err(StartupFailure::into_error)
        .context("project, CLI, or environment override is unusable")?;
    Ok((reloaded, client))
}

fn take_form(slot: Option<ModelMenu>) -> Option<ProviderForm> {
    match slot {
        Some(ModelMenu::Form(form)) => Some(form),
        _ => None,
    }
}

fn render(frame: &mut ratatui::Frame, form: &ProviderForm) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let header_h = 6.min(area.height);
    let header = Rect::new(area.x, area.y, area.width, header_h);
    let path = Config::global_config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.opencoder/config.json".into());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Welcome to OpenCoder — configure your first model",
                Style::default()
                    .fg(crate::theme::accent())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::raw(format!(" Settings will be saved to {path}")),
            Line::raw(" Fill provider/model/base URL/API key, then select [Save]."),
            Line::raw(" API key accepts a literal secret or an ENV_VAR name. Esc/Ctrl-D exits."),
        ]),
        header,
    );
    crate::model_menu::render_model_popup(
        frame,
        area,
        area.y.saturating_add(area.height),
        &ModelMenu::Form(form.clone()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::{scoped_config_home, ProviderConfig};
    use std::collections::HashMap;

    fn ready_config() -> Config {
        let mut providers = HashMap::new();
        providers.insert(
            "demo".into(),
            ProviderConfig {
                base_url: "https://example.com/v1".into(),
                api_key: Some("sk-onboarding-secret-1234".into()),
                model: Some("model-x".into()),
                headers: Vec::new(),
            },
        );
        Config {
            model: "demo/model-x".into(),
            providers,
            ..Config::default()
        }
    }

    #[test]
    fn readiness_rejects_missing_key_and_invalid_url() {
        let missing = Config::default();
        let failure = build_ready_client(&missing)
            .err()
            .expect("missing key must fail");
        match failure {
            // Missing provider/api_key is exactly what the wizard fixes.
            StartupFailure::Credentials(error) => {
                assert!(error.to_string().contains("credentials"));
            }
            StartupFailure::Unbuildable(_) => {
                panic!("missing key must classify as wizard-fixable Credentials")
            }
        }

        let mut invalid = ready_config();
        invalid.providers.get_mut("demo").unwrap().base_url = "not a url".into();
        let failure = build_ready_client(&invalid)
            .err()
            .expect("invalid URL must fail");
        match failure {
            StartupFailure::Unbuildable(error) => {
                assert!(error.to_string().contains("base_url"));
            }
            StartupFailure::Credentials(_) => {
                panic!("unparseable base_url must classify as Unbuildable")
            }
        }
    }

    #[test]
    fn readiness_accepts_complete_config_without_network_call() {
        assert!(build_ready_client(&ready_config()).is_ok());
    }

    /// Regression (defect #8): an endpoint parsed but an unbuildable client
    /// (invalid proxy env) must NOT be classified as wizard-fixable — the
    /// wizard cannot fix a proxy, so routing it there traps the user.
    #[test]
    fn startup_failure_classifies_invalid_proxy_as_unbuildable() {
        let mut config = ready_config();
        config.network.proxy = Some("::not a proxy::".into());
        let failure = build_ready_client(&config)
            .err()
            .expect("invalid proxy must fail client construction");
        match failure {
            StartupFailure::Unbuildable(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("proxy") || text.contains("http client"),
                    "unexpected proxy failure text: {text}"
                );
            }
            StartupFailure::Credentials(_) => {
                panic!("invalid proxy must classify as Unbuildable, not Credentials")
            }
        }
    }

    /// Non-http(s) base_url schemes reach the app, not the wizard.
    #[test]
    fn startup_failure_classifies_non_http_base_url_as_unbuildable() {
        let mut config = ready_config();
        config.providers.get_mut("demo").unwrap().base_url = "ftp://x".into();
        let failure = build_ready_client(&config)
            .err()
            .expect("non-http(s) base_url must fail");
        match failure {
            StartupFailure::Unbuildable(error) => {
                assert!(error.to_string().contains("http(s)"));
            }
            StartupFailure::Credentials(_) => {
                panic!("non-http(s) base_url must classify as Unbuildable")
            }
        }
    }

    #[test]
    fn unbuildable_client_fails_every_stream_with_reason() {
        use opencoder_llm::{ChatRequest, ChatStream};

        let client = UnbuildableClient {
            reason: "invalid proxy '::not a proxy::'".into(),
        };
        let request = ChatRequest {
            model: "demo/model-x".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: None,
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            cache_salt: None,
        };
        let error = client
            .chat_stream(request)
            .expect_err("unbuildable client must refuse every turn");
        let text = error.to_string();
        assert!(text.contains("model client unavailable"), "got: {text}");
        assert!(text.contains("invalid proxy"), "got: {text}");

        let stream: &dyn ChatStream = &client;
        assert_eq!(stream.backend(), "unavailable");
    }

    #[test]
    fn onboarding_form_prefills_effective_model_and_focuses_secret() {
        let form = ProviderForm::new_onboarding(&ready_config());
        assert_eq!(form.name, "demo");
        assert_eq!(form.model_id, "model-x");
        assert_eq!(form.base_url, "https://example.com/v1");
        assert_eq!(form.focus, crate::model_menu::ProviderField::ApiKey);
        assert!(!form.api_key_display().contains("sk-onboarding-secret-1234"));
    }

    #[test]
    fn onboarding_escape_and_ctrl_d_exit_without_saving() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        for key in [
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        ] {
            let mut slot = Some(ModelMenu::Form(ProviderForm::new_onboarding(
                &Config::default(),
            )));
            let outcome = handle_model_key(&mut slot, key);
            assert!(matches!(outcome, ModelOutcome::Cancel | ModelOutcome::Quit));
            assert!(slot.is_none());
        }
    }

    #[test]
    fn global_patch_builds_locally_ready_candidate() {
        let home = tempfile::tempdir().unwrap();
        let _isolation = scoped_config_home(home.path().to_path_buf());
        let patch = serde_json::json!({
            "model": "demo/model-x",
            "providers": {"demo": {
                "base_url": "https://example.com/v1",
                "api_key": "secret",
                "model": "model-x",
                "headers": []
            }}
        });
        let candidate = Config::default().merged_with(&patch);
        assert!(build_ready_client(&candidate).is_ok());
    }

    #[test]
    fn finalize_persists_global_config_and_returns_effective_model() {
        let home = tempfile::tempdir().unwrap();
        let _isolation = scoped_config_home(home.path().to_path_buf());
        let workdir = tempfile::tempdir().unwrap();
        let patch = serde_json::json!({
            "model": "demo/model-x",
            "providers": {"demo": {
                "base_url": "https://example.com/v1",
                "api_key": "secret",
                "model": "model-x",
                "headers": []
            }}
        });

        let (config, _) =
            finalize_config(workdir.path(), None, &Config::default(), &patch).unwrap();
        assert_eq!(config.model, "demo/model-x");
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".opencoder/config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["providers"]["demo"]["model"], "model-x");
    }

    #[test]
    fn finalize_rejects_unusable_project_override_after_global_save() {
        let home = tempfile::tempdir().unwrap();
        let _isolation = scoped_config_home(home.path().to_path_buf());
        let workdir = tempfile::tempdir().unwrap();
        std::fs::write(
            workdir.path().join("opencoder.json"),
            r#"{"model":"other/model-y"}"#,
        )
        .unwrap();
        let patch = serde_json::json!({
            "model": "demo/model-x",
            "providers": {"demo": {
                "base_url": "https://example.com/v1",
                "api_key": "secret",
                "model": "model-x"
            }}
        });

        let error = finalize_config(workdir.path(), None, &Config::default(), &patch)
            .err()
            .expect("project override must remain authoritative");
        assert!(error.to_string().contains("override"));
    }

    #[test]
    fn onboarding_renderer_shows_guidance_and_masks_secret() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let home = tempfile::tempdir().unwrap();
        let _isolation = scoped_config_home(home.path().to_path_buf());
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
        let form = ProviderForm::new_onboarding(&ready_config());
        terminal.draw(|frame| render(frame, &form)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("configure your first model"));
        assert!(text.contains(".opencoder/config.json"));
        assert!(!text.contains("sk-onboarding-secret-1234"));
    }
}
