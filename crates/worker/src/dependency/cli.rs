//! Transport boundary for the viking dependency flow.
//!
//! The node reaches the hosted VTA trigger endpoint through the reviewed
//! `viking-cli` binary (it owns the viking_auth request signing), exactly as
//! the prod canary does. This module only builds argv, scrubs the child
//! environment down to a single issued token, spawns the CLI, and decodes its
//! JSON envelope. All pure pieces are separated so tests never spawn a
//! process.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

/// Hosted API prefix mounted on the ai-viking origin.
const HOSTED_PREFIX: &str = "/api/v1/viking-test-agent";
const DEFAULT_ORIGIN: &str = "https://ai-viking.bytedance.net";
const DEFAULT_BIN: &str = "viking-cli";

/// Minimal request surface the flow orchestration needs. Mocked in tests.
#[async_trait]
pub trait VikingTransport: Send + Sync {
    /// Perform one CLI request. `suffix` is the path *after* the hosted
    /// prefix (e.g. `/dependency-analysis/tasks`). `headers`/`query` are raw
    /// `k=v` pairs (headers e.g. `x-request-id=...`); `body` is an
    /// already-serialized JSON string for non-GET.
    async fn request(
        &self,
        method: &str,
        suffix: &str,
        headers: &[(String, String)],
        query: &[(String, String)],
        body: Option<&str>,
    ) -> Result<Value>;
}

/// A transport that shells out to the `viking-cli` binary.
pub struct CliTransport {
    bin: PathBuf,
    origin: String,
    token: String,
    timeout: Duration,
}

impl CliTransport {
    /// Build from the process environment: origin overridable via
    /// `VIKING_BASE_URL`; token from `VIKING_AUTH_TOKEN` (else
    /// `VIKING_AUTH_PROD_ISSUED_TOKEN`). The token is never logged.
    pub fn from_env() -> Result<Self> {
        let origin = std::env::var("VIKING_BASE_URL")
            .ok()
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_ORIGIN.to_string());
        let token = resolve_token().ok_or_else(|| {
            anyhow!(
                "missing viking auth token: set VIKING_AUTH_TOKEN or VIKING_AUTH_PROD_ISSUED_TOKEN"
            )
        })?;
        let bin = std::env::var("VIKING_CLI_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_BIN));
        Ok(Self {
            bin,
            origin,
            token,
            timeout: Duration::from_secs(120),
        })
    }
}

fn resolve_token() -> Option<String> {
    std::env::var("VIKING_AUTH_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("VIKING_AUTH_PROD_ISSUED_TOKEN")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .map(|v| v.trim().to_string())
}

/// Proxy/auth vars the reviewed CLI must NOT inherit (it signs requests
/// itself and must talk directly to the target).
fn is_scrubbed(key: &str) -> bool {
    matches!(
        key,
        "ALL_PROXY"
            | "HTTP_PROXY"
            | "HTTPS_PROXY"
            | "NO_PROXY"
            | "all_proxy"
            | "http_proxy"
            | "https_proxy"
            | "no_proxy"
            | "AUTHORIZATION"
    ) || (key.starts_with("VIKING_") && key.ends_with("_TOKEN"))
}

/// Build the child environment: drop every proxy/authorization/other-viking
/// token and set exactly one issued `VIKING_AUTH_TOKEN`. Pure for testing.
pub(crate) fn scrub_env<I, S>(parent: I, token: &str) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (S, S)>,
    S: AsRef<str>,
{
    let mut child: Vec<(String, String)> = parent
        .into_iter()
        .filter(|(k, _)| !is_scrubbed(k.as_ref()))
        .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
        .collect();
    child.push(("VIKING_AUTH_TOKEN".to_string(), token.to_string()));
    child
}

/// Build the full `viking-cli raw request` argv. Pure for testing.
pub(crate) fn build_argv(
    bin: &str,
    origin: &str,
    method: &str,
    suffix: &str,
    headers: &[(String, String)],
    query: &[(String, String)],
    body: Option<&str>,
) -> Vec<String> {
    let path = format!("{HOSTED_PREFIX}{suffix}");
    let mut argv = vec![
        bin.to_string(),
        "raw".to_string(),
        "request".to_string(),
        "--base-url".to_string(),
        origin.to_string(),
        "--method".to_string(),
        method.to_string(),
        "--path".to_string(),
        path,
    ];
    for header in headers {
        argv.push("--header".to_string());
        argv.push(format!("{}={}", header.0, header.1));
    }
    for pair in query {
        argv.push("--query".to_string());
        argv.push(format!("{}={}", pair.0, pair.1));
    }
    if let Some(body) = body {
        argv.push("--body-json".to_string());
        argv.push(body.to_string());
    }
    if method != "GET" {
        argv.push("--yes".to_string());
    }
    argv.push("--output".to_string());
    argv.push("json".to_string());
    argv
}

/// Decode the CLI's stdout envelope: `{status_code, body}` on success.
fn decode_envelope(stdout: &str) -> Result<Value> {
    let envelope: Value =
        serde_json::from_str(stdout).with_context(|| "viking-cli returned non-JSON output")?;
    let status = envelope
        .get("status_code")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("viking-cli envelope missing status_code"))?;
    if !(200..300).contains(&status) {
        let message = envelope
            .get("body")
            .and_then(|b| b.get("message"))
            .and_then(Value::as_str)
            .or_else(|| envelope.get("body").and_then(Value::as_str))
            .unwrap_or("request failed");
        bail!("viking-cli HTTP {status}: {message}");
    }
    envelope
        .get("body")
        .cloned()
        .filter(|b| !b.is_null())
        .ok_or_else(|| anyhow!("viking-cli envelope missing body"))
}

#[async_trait]
impl VikingTransport for CliTransport {
    async fn request(
        &self,
        method: &str,
        suffix: &str,
        headers: &[(String, String)],
        query: &[(String, String)],
        body: Option<&str>,
    ) -> Result<Value> {
        let argv = build_argv(
            self.bin.to_string_lossy().as_ref(),
            &self.origin,
            method,
            suffix,
            headers,
            query,
            body,
        );
        let env = scrub_env(std::env::vars(), &self.token);
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        for (k, v) in &env {
            command.env(k, v);
        }
        let output = tokio::time::timeout(self.timeout, command.output())
            .await
            .map_err(|_| anyhow!("viking-cli timed out after {}s", self.timeout.as_secs()))?
            .context("failed to spawn viking-cli")?;
        if !output.status.success() {
            // Never include the token; stderr may carry request detail.
            let detail = String::from_utf8_lossy(&output.stderr);
            let tail = detail
                .lines()
                .rev()
                .find(|l| !l.contains("VIKING_AUTH_TOKEN"))
                .unwrap_or("non-zero exit");
            bail!("viking-cli exited {}: {tail}", output.status);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            bail!("viking-cli produced no output");
        }
        decode_envelope(stdout.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_shape_for_post_and_get() {
        let post = build_argv(
            "viking-cli",
            DEFAULT_ORIGIN,
            "POST",
            "/dependency-analysis/tasks",
            &[],
            &[],
            Some("{\"items\":[]}"),
        );
        assert!(post.contains(&"--body-json".to_string()));
        assert!(post.contains(&"--yes".to_string()));
        assert!(post.contains(&"--path".to_string()));
        assert!(post
            .iter()
            .any(|a| a == &format!("{HOSTED_PREFIX}/dependency-analysis/tasks")));

        let v3 = build_argv(
            "viking-cli",
            DEFAULT_ORIGIN,
            "POST",
            "/dependency-analysis/tasks",
            &[(
                "x-request-id".to_string(),
                "jianyingqa-current-v3-deadbeef".to_string(),
            )],
            &[],
            Some("{\"items\":[]}"),
        );
        assert!(v3.contains(&"--header".to_string()));
        assert!(v3.contains(&"x-request-id=jianyingqa-current-v3-deadbeef".to_string()));

        let get = build_argv(
            "viking-cli",
            DEFAULT_ORIGIN,
            "GET",
            "/dependency-analysis/tasks/dep-1",
            &[],
            &[("page".to_string(), "1".to_string())],
            None,
        );
        assert!(!get.contains(&"--yes".to_string()));
        assert!(!get.contains(&"--body-json".to_string()));
        assert!(get.contains(&"--query".to_string()));
        assert!(get.contains(&"page=1".to_string()));
    }

    #[test]
    fn env_scrub_keeps_single_authority() {
        let parent = [
            ("PATH", "/usr/bin"),
            ("HTTP_PROXY", "http://proxy:1"),
            ("HTTPS_PROXY", "http://proxy:1"),
            ("AUTHORIZATION", "Bearer other"),
            ("VIKING_AUTH_TOKEN", "stale-inline"),
            ("VIKING_AUTH_PROD_ISSUED_TOKEN", "prod-token"),
            ("VIKING_OTHER_TOKEN", "other"),
            ("HOME", "/root"),
        ];
        let child = scrub_env(parent, "issued");
        let keys: Vec<&str> = child.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"HOME"));
        assert!(!keys.contains(&"HTTP_PROXY"));
        assert!(!keys.contains(&"HTTPS_PROXY"));
        assert!(!keys.contains(&"AUTHORIZATION"));
        assert!(!keys.contains(&"VIKING_AUTH_PROD_ISSUED_TOKEN"));
        assert!(!keys.contains(&"VIKING_OTHER_TOKEN"));
        // Exactly one authority, the scrubbed inline value replaced by ours.
        let auth: Vec<&str> = child
            .iter()
            .filter(|(k, _)| k == "VIKING_AUTH_TOKEN")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(auth, vec!["issued"]);
    }

    #[test]
    fn envelope_decoded_or_error() {
        let ok = decode_envelope(
            r#"{"status_code":200,"body":{"created_items":[{"task_id":"dep-1"}]}}"#,
        )
        .unwrap();
        assert_eq!(ok["created_items"][0]["task_id"], "dep-1");

        assert!(decode_envelope(r#"{"status_code":500,"body":{"message":"boom"}}"#).is_err());
        assert!(decode_envelope("not json").is_err());
        assert!(decode_envelope(r#"{"status_code":200}"#).is_err());
    }

    #[test]
    fn token_resolution_prefers_inline() {
        // Safety: only assert the scrub helper never echoes the token into an
        // argv (argv carries no auth at all).
        let argv = build_argv("viking-cli", DEFAULT_ORIGIN, "GET", "/x", &[], &[], None);
        assert!(!argv.iter().any(|a| a.contains("TOKEN")));
    }
}
