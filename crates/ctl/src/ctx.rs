//! Connection context resolution: `--server`/`--token`/`--token-file` flags
//! override the `OPENCODER_SERVER_URL` / `OPENCODER_SERVER_TOKEN` environment
//! (same token variable the server and agent binaries already honor).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::http;

pub struct Ctx {
    /// Server base URL, normalized without a trailing slash.
    pub server: String,
    pub token: String,
    pub verbose: bool,
    pub http: reqwest::Client,
}

fn clean_url(value: &str, source: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    anyhow::ensure!(!trimmed.is_empty(), "{source} contains an empty server URL");
    anyhow::ensure!(
        trimmed.starts_with("http://") || trimmed.starts_with("https://"),
        "{source} must be an http(s) URL, got {trimmed:?}"
    );
    Ok(trimmed.to_owned())
}

fn token_value(value: String, source: &str) -> Result<String> {
    let token = value.trim();
    anyhow::ensure!(!token.is_empty(), "{source} contains an empty token");
    Ok(token.to_owned())
}

/// Pure resolution over explicit inputs plus an environment lookup function
/// (injectable for tests). Flags win; env vars are the fallback.
pub fn resolve_with_env(
    server: Option<&str>,
    token: Option<&str>,
    token_file: Option<&Path>,
    verbose: bool,
    env: impl Fn(&str) -> Option<String>,
) -> Result<(String, String)> {
    let _ = verbose;
    let server = match server {
        Some(value) => clean_url(value, "--server")?,
        None => match env("OPENCODER_SERVER_URL") {
            Some(value) => clean_url(&value, "OPENCODER_SERVER_URL")?,
            None => anyhow::bail!("server URL required: pass --server or set OPENCODER_SERVER_URL"),
        },
    };
    anyhow::ensure!(
        token.is_none() || token_file.is_none(),
        "--token and --token-file are mutually exclusive"
    );
    let token = if let Some(value) = token {
        token_value(value.to_owned(), "--token")?
    } else if let Some(path) = token_file {
        let value = std::fs::read_to_string(path)
            .with_context(|| format!("read token file {}", path.display()))?;
        token_value(value, "token file")?
    } else {
        match env("OPENCODER_SERVER_TOKEN") {
            Some(value) => token_value(value, "OPENCODER_SERVER_TOKEN")?,
            None => anyhow::bail!(
                "bearer token required: pass --token, --token-file, or set OPENCODER_SERVER_TOKEN"
            ),
        }
    };
    Ok((server, token))
}

pub fn resolve(
    server: Option<&str>,
    token: Option<&str>,
    token_file: Option<PathBuf>,
    verbose: bool,
) -> Result<Ctx> {
    let (server, token) = resolve_with_env(server, token, token_file.as_deref(), verbose, |key| {
        std::env::var(key).ok().filter(|v| !v.trim().is_empty())
    })?;
    Ok(Ctx {
        server,
        token,
        verbose,
        http: http::client()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn flags_win_over_env() {
        let env = |k: &str| match k {
            "OPENCODER_SERVER_URL" => Some("http://env:9000/".into()),
            "OPENCODER_SERVER_TOKEN" => Some("env-token".into()),
            _ => None,
        };
        let (server, token) = resolve_with_env(
            Some("http://flag:8080/"),
            Some("flag-token"),
            None,
            false,
            env,
        )
        .unwrap();
        assert_eq!(server, "http://flag:8080");
        assert_eq!(token, "flag-token");
    }

    #[test]
    fn env_fallback_and_trailing_slash_trim() {
        let env = |k: &str| match k {
            "OPENCODER_SERVER_URL" => Some("http://env:9000//".into()),
            "OPENCODER_SERVER_TOKEN" => Some("env-token".into()),
            _ => None,
        };
        let (server, token) = resolve_with_env(None, None, None, false, env).unwrap();
        assert_eq!(server, "http://env:9000");
        assert_eq!(token, "env-token");
    }

    #[test]
    fn missing_server_or_token_is_an_error() {
        assert!(resolve_with_env(None, Some("t"), None, false, no_env).is_err());
        assert!(resolve_with_env(Some("http://x"), None, None, false, no_env).is_err());
    }

    #[test]
    fn token_and_token_file_conflict() {
        assert!(resolve_with_env(
            Some("http://x"),
            Some("t"),
            Some(Path::new("/tmp/f")),
            false,
            no_env
        )
        .is_err());
    }

    #[test]
    fn non_http_scheme_rejected() {
        assert!(resolve_with_env(Some("ftp://x"), Some("t"), None, false, no_env).is_err());
    }
}
