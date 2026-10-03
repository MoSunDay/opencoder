//! User-configured TUI event commands from ~/.opencoder/hooks.json.

use std::path::Path;
use std::time::Duration;

use opencoder_core::Config;
use serde::Deserialize;

#[derive(Clone, Copy)]
pub(crate) enum Event {
    TurnDone,
    Question,
}

#[derive(Default, Deserialize)]
struct Hooks {
    #[serde(default)]
    turn_done: Vec<String>,
    #[serde(default)]
    question: Vec<String>,
}

impl Hooks {
    fn commands(&self, event: Event) -> &[String] {
        match event {
            Event::TurnDone => &self.turn_done,
            Event::Question => &self.question,
        }
    }
}

async fn load(path: &Path) -> Result<Hooks, Box<dyn std::error::Error + Send + Sync>> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Hooks::default()),
        Err(error) => return Err(error.into()),
    };
    Ok(serde_json::from_slice(&bytes)?)
}

async fn run(command: &str) -> anyhow::Result<std::process::ExitStatus> {
    Ok(
        opencoder_session::tools::command::host::run(command, None, Duration::from_secs(3))
            .await?
            .status,
    )
}

pub(crate) fn emit(event: Event) {
    let Ok(path) = Config::global_config_path() else {
        return;
    };
    tokio::spawn(async move {
        let hooks = match load(&path.with_file_name("hooks.json")).await {
            Ok(hooks) => hooks,
            Err(error) => {
                tracing::debug!(%error, "TUI hooks unavailable");
                return;
            }
        };
        for command in hooks
            .commands(event)
            .iter()
            .filter(|s| !s.trim().is_empty())
        {
            match run(command).await {
                Ok(status) if status.success() => {}
                Ok(status) => tracing::debug!(%status, "TUI hook rejected"),
                Err(error) => tracing::debug!(%error, "TUI hook unavailable"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn configured_events_select_commands_and_execute() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("hooks.json");
        let output = dir.path().join("event");
        std::fs::write(
            &config,
            format!(
                "{{\"turn_done\":[\"printf done > '{}'\"],\"question\":[\"printf question > '{}'\"]}}",
                output.display(),
                output.display()
            ),
        )
        .unwrap();
        let hooks = load(&config).await.unwrap();
        assert_eq!(hooks.commands(Event::TurnDone).len(), 1);
        assert_eq!(hooks.commands(Event::Question).len(), 1);
        assert!(run(&hooks.commands(Event::Question)[0])
            .await
            .unwrap()
            .success());
        assert_eq!(std::fs::read_to_string(output).unwrap(), "question");
    }

    #[tokio::test]
    async fn missing_config_has_no_hooks_and_invalid_json_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("hooks.json");
        assert!(load(&config)
            .await
            .unwrap()
            .commands(Event::TurnDone)
            .is_empty());
        std::fs::write(&config, "{").unwrap();
        assert!(load(&config).await.is_err());
    }

    #[tokio::test]
    async fn emitted_question_uses_the_current_config_home() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join(".opencoder");
        std::fs::create_dir(&config_dir).unwrap();
        let output = dir.path().join("question-event");
        #[cfg(windows)]
        opencoder_session::process::configure_supervisor_binary(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("opencoder.exe"),
        )
        .unwrap();
        let command = if cfg!(windows) {
            format!(
                "Set-Content -LiteralPath '{}' -Value ready -NoNewline",
                output.display()
            )
        } else {
            format!("printf ready > '{}'", output.display())
        };
        #[cfg(windows)]
        {
            assert!(run(&command)
                .await
                .expect("native hook command failed")
                .success());
            std::fs::remove_file(&output).unwrap();
        }
        std::fs::write(
            config_dir.join("hooks.json"),
            serde_json::json!({"question": [command]}).to_string(),
        )
        .unwrap();
        let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
        emit(Event::Question);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !output.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("question hook did not run");
        assert_eq!(std::fs::read_to_string(output).unwrap(), "ready");
    }
}
