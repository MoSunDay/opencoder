//! The booted platform server's credential surface, one real binary per
//! case (port 0, isolated tempdir workdir, scrubbed env):
//! - a `--token-file` with a trailing newline trims into a working Bearer;
//! - `OPENCODER_SERVER_TOKEN` alone is a valid credential source;
//! - `--auth-enabled false` drops the Bearer requirement (and reports it);
//! - a separate metrics token authenticates `GET /metrics` ONLY;

use std::path::Path;

use crate::proc::ServerProc;
use crate::support::http_util::{http, http_text};

const TOKEN: &str = "modes-e2e-server-token";
const METRICS_TOKEN: &str = "modes-e2e-metrics-token";

/// Boot the platform server on an OS-picked port and return its base URL.
fn boot(workdir: &Path, args: &[&str], envs: &[(&str, String)]) -> (ServerProc, String) {
    let workdir = workdir.to_path_buf();
    let data = workdir.join("data");
    let mut full = vec![
        "--workdir",
        workdir.to_str().unwrap(),
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--data-dir",
        data.to_str().unwrap(),
    ];
    full.extend_from_slice(args);
    let proc = ServerProc::spawn(&workdir, &full, envs);
    let base = proc.wait_http_ready(60);
    (proc, base)
}

#[test]
fn token_file_boot_enforces_bearer() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("server.token");
    // Trailing newline is the normal credential-file shape; the server trims.
    std::fs::write(&file, format!("{TOKEN}\n")).unwrap();
    let (_server, base) = boot(
        tmp.path(),
        &["--token-file", file.to_str().unwrap()],
        &[],
    );

    let (status, _) = http(&base, "GET", "/api/health", "", "");
    assert_eq!(status, 401, "missing Bearer must be refused");
    let (status, body) = http(&base, "GET", "/api/health", TOKEN, "");
    assert_eq!(status, 200, "file token must authenticate: {body}");
    assert_eq!(body["ok"], serde_json::json!(true));
}

#[test]
fn env_only_token_boots() {
    let tmp = tempfile::tempdir().unwrap();
    let (_server, base) = boot(tmp.path(), &[], &[("OPENCODER_SERVER_TOKEN", TOKEN.into())]);

    let (status, _) = http(&base, "GET", "/api/health", "", "");
    assert_eq!(status, 401, "missing Bearer must be refused");
    let (status, body) = http(&base, "GET", "/api/health", TOKEN, "");
    assert_eq!(status, 200, "env token must authenticate: {body}");
}

#[test]
fn auth_disabled_serves_without_bearer() {
    let tmp = tempfile::tempdir().unwrap();
    // `-v` exercises the verbose logging bootstrap on the same boot.
    let (_server, base) = boot(
        tmp.path(),
        &["--token", TOKEN, "--auth-enabled", "false", "-v"],
        &[],
    );

    let (status, body) = http(&base, "GET", "/api/health", "", "");
    assert_eq!(status, 200, "no Bearer needed when auth is off: {body}");
    let (status, body) = http(&base, "GET", "/api/auth/status", "", "");
    assert_eq!(status, 200);
    assert_eq!(body, serde_json::json!({"enabled": false}));
    // The exempt readiness endpoint stays unsigned in every mode.
    let (status, body) = http(&base, "GET", "/api/time", "", "");
    assert_eq!(status, 200, "{body}");
}

#[test]
fn metrics_token_scoped_to_metrics_only() {
    let tmp = tempfile::tempdir().unwrap();
    let server_file = tmp.path().join("server.token");
    std::fs::write(&server_file, format!("{TOKEN}\n")).unwrap();
    let metrics_file = tmp.path().join("metrics.token");
    std::fs::write(&metrics_file, format!("{METRICS_TOKEN}\n")).unwrap();
    let (_server, base) = boot(
        tmp.path(),
        &[
            "--token-file",
            server_file.to_str().unwrap(),
            "--metrics-token-file",
            metrics_file.to_str().unwrap(),
        ],
        &[],
    );

    // Both credentials open the metrics endpoint (prometheus text format).
    let (status, body) = http_text(&base, "GET", "/metrics", METRICS_TOKEN, &[], None);
    assert_eq!(status, 200, "metrics token must open /metrics: {body}");
    assert!(
        body.contains("# TYPE"),
        "prometheus exposition must carry metric types: {body}"
    );
    let (status, _) = http(&base, "GET", "/metrics", TOKEN, "");
    assert_eq!(status, 200, "server token must open /metrics");
    let (status, _) = http(&base, "GET", "/metrics", "", "");
    assert_eq!(status, 401, "anonymous /metrics must be refused");

    // The metrics token NEVER authorizes the API surface (auth_mw answers
    // 401, not 403, for a metrics token outside GET /metrics).
    let (status, body) = http(&base, "GET", "/api/health", METRICS_TOKEN, "");
    assert_eq!(status, 401, "metrics token must not open /api/*: {body}");
    let (status, body) = http(&base, "GET", "/api/health", TOKEN, "");
    assert_eq!(status, 200, "server token still opens /api/*: {body}");
}

#[test]
fn web_flag_keeps_spa_and_api_mounted() {
    let tmp = tempfile::tempdir().unwrap();
    let (_server, base) = boot(tmp.path(), &["--token", TOKEN, "--web", "true"], &[]);

    let (status, body) = http_text(&base, "GET", "/", "", &[], None);
    assert_eq!(status, 200, "SPA shell stays mounted: {body}");
    assert!(
        body.to_lowercase().contains("<!doctype html>"),
        "root must serve the SPA shell, got: {body}"
    );
    // The unsigned readiness endpoint answers regardless of the web mount.
    let (status, body) = http(&base, "GET", "/api/time", "", "");
    assert_eq!(status, 200, "{body}");
}

/// `--web false` prunes the SPA mount (the session-relay fallback owns the
/// bare root) while every API route — signed and unsigned — stays served.
#[test]
fn web_disabled_drops_spa_keeps_api() {
    let tmp = tempfile::tempdir().unwrap();
    let (_server, base) = boot(tmp.path(), &["--token", TOKEN, "--web", "false"], &[]);

    // The SPA shell is gone: the bare root is NOT html any more.
    let (status, body) = http_text(&base, "GET", "/", "", &[], None);
    assert_ne!(status, 200, "root must not serve the SPA: {status} {body}");
    assert!(
        !body.to_lowercase().contains("<!doctype html>"),
        "root must not leak the SPA shell, got: {body}"
    );
    let (status, body) = http_text(&base, "GET", "/static/app.js", "", &[], None);
    assert_ne!(status, 200, "static assets must be unmounted: {status} {body}");
    // API surface intact: unsigned readiness + a bearer-gated route.
    let (status, body) = http(&base, "GET", "/api/time", "", "");
    assert_eq!(status, 200, "{body}");
    let (status, _) = http(&base, "GET", "/api/health", "", "");
    assert_eq!(status, 401, "auth gate survives the web pruning");
    let (status, body) = http(&base, "GET", "/api/health", TOKEN, "");
    assert_eq!(status, 200, "{body}");
}
