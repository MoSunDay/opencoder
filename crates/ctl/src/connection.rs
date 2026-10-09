//! File-backed CLI defaults. Credentials stay in local files, outside Git.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub server: Option<String>,
    pub token: Option<String>,
    pub token_file: Option<PathBuf>,
    #[serde(default)]
    pub verbose: bool,
}

pub fn default_path(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    env("XDG_CONFIG_HOME")
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|root| root.join("opencoder/ctl.json"))
}

pub fn load(explicit: Option<&Path>, default: Option<&Path>) -> Result<Defaults> {
    let Some(path) = explicit.or(default) else {
        return Ok(Defaults::default());
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if explicit.is_none() && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Defaults::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("read CLI configuration {}", path.display()))
        }
    };
    let mut defaults: Defaults = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse CLI configuration {}", path.display()))?;
    anyhow::ensure!(
        defaults.token.is_none() || defaults.token_file.is_none(),
        "CLI configuration token and token_file are mutually exclusive"
    );
    if let Some(file) = defaults.token_file.as_mut() {
        if file.is_relative() {
            *file = path.parent().unwrap_or(Path::new(".")).join(&*file);
        }
    }
    Ok(defaults)
}

pub fn merge(
    server: Option<&str>,
    token: Option<&str>,
    token_file: Option<&Path>,
    verbose: bool,
    defaults: Defaults,
    env: impl Fn(&str) -> Option<String>,
) -> Defaults {
    let server = server
        .map(str::to_owned)
        .or_else(|| env("OPENCODER_SERVER_URL"))
        .or(defaults.server);
    let (token, token_file) = if token.is_some() || token_file.is_some() {
        (token.map(str::to_owned), token_file.map(Path::to_path_buf))
    } else if let Some(token) = env("OPENCODER_SERVER_TOKEN") {
        (Some(token), None)
    } else {
        (defaults.token, defaults.token_file)
    };
    Defaults {
        server,
        token,
        token_file,
        verbose: verbose || defaults.verbose,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_resolves_relative_credentials_and_rejects_ambiguous_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ctl.json");
        assert!(load(Some(&path), None).is_err());
        assert!(load(None, Some(&path)).unwrap().server.is_none());
        std::fs::write(
            &path,
            r#"{"server":"http://localhost","token_file":"admin.text"}"#,
        )
        .unwrap();
        let config = load(Some(&path), None).unwrap();
        assert_eq!(config.token_file, Some(dir.path().join("admin.text")));
        std::fs::write(&path, r#"{"token":"a","token_file":"b"}"#).unwrap();
        assert!(load(Some(&path), None).is_err());
        std::fs::write(&path, r#"{"tokne":"a"}"#).unwrap();
        assert!(load(Some(&path), None).is_err());
    }

    #[test]
    fn flags_then_environment_then_file_choose_one_credential_source() {
        let config = || Defaults {
            server: Some("http://file".into()),
            token_file: Some("file-token".into()),
            verbose: true,
            ..Defaults::default()
        };
        let env = |name: &str| {
            Some(
                if name == "OPENCODER_SERVER_URL" {
                    "http://env"
                } else {
                    "env-token"
                }
                .into(),
            )
        };
        let flags = merge(
            Some("http://flag"),
            Some("flag-token"),
            None,
            false,
            config(),
            env,
        );
        assert_eq!(flags.server.as_deref(), Some("http://flag"));
        assert_eq!(flags.token.as_deref(), Some("flag-token"));
        assert!(flags.token_file.is_none());
        assert!(flags.verbose);
        let environment = merge(None, None, None, false, config(), env);
        assert_eq!(environment.server.as_deref(), Some("http://env"));
        assert_eq!(environment.token.as_deref(), Some("env-token"));
        assert!(environment.token_file.is_none());
        let file = merge(None, None, None, false, config(), |_| None);
        assert_eq!(file.server.as_deref(), Some("http://file"));
        assert_eq!(file.token_file, Some("file-token".into()));
    }

    #[test]
    fn standard_config_location_uses_xdg_then_home() {
        assert_eq!(
            default_path(|key| (key == "XDG_CONFIG_HOME").then(|| "/config".into())),
            Some("/config/opencoder/ctl.json".into())
        );
        assert_eq!(
            default_path(|key| (key == "HOME").then(|| "/home/test".into())),
            Some("/home/test/.config/opencoder/ctl.json".into())
        );
    }
}
