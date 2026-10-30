//! The fail-fast rejection matrix: every misconfigured startup must exit
//! non-zero BEFORE binding a port, with a stable stderr message an
//! operator can act on. One test walks a table so each run stays a plain
//! `Command::output()` (no boot, no polling).

use std::process::Command;

use crate::proc::server_bin;

/// Environment hygiene shared by every rejection run: a developer shell
/// exporting `OPENCODER_SERVER_TOKEN` must not mask the missing-token
/// case, and proxies must never divert loopback probes.
fn reject(args: &[String]) -> (bool, String) {
    let output = Command::new(server_bin())
        .args(args)
        .env_remove("OPENCODER_SERVER_TOKEN")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .output()
        .expect("run opencoder-server");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn invalid_startup_flags_fail_fast_before_binding() {
    let tmp = tempfile::tempdir().unwrap();
    // Credential fixtures: whitespace-only files are rejected, trailing
    // newlines in real files are the normal operator format.
    let empty = tmp.path().join("empty.token");
    std::fs::write(&empty, "   \n").unwrap();
    let token = tmp.path().join("server.token");
    std::fs::write(&token, "server-secret\n").unwrap();
    let metrics_same = tmp.path().join("metrics-same.token");
    std::fs::write(&metrics_same, "server-secret\n").unwrap();
    let metrics_empty = tmp.path().join("metrics-empty.token");
    std::fs::write(&metrics_empty, "\n").unwrap();
    let missing = tmp.path().join("missing.token");
    let release = tmp.path().join("release.json");
    let path = |p: &std::path::Path| p.to_str().unwrap().to_string();

    let cases: Vec<(&str, Vec<String>, &str)> = vec![
        (
            "token flag and token file are mutually exclusive",
            strings(["--token", "a", "--token-file", &path(&token)]),
            "cannot be used with '--token-file <PATH>'",
        ),
        (
            "resource service conflicts with release config",
            strings(["--resources", "--release-config", &path(&release)]),
            "cannot be used with '--release-config <RELEASE_CONFIG>'",
        ),
        (
            "todo-resources conflicts with resources",
            strings(["--todo-resources", "--resources", "--token", "t"]),
            "cannot be used with '--resources'",
        ),
        (
            "todo resource service requires a data dir",
            strings(["--todo-resources", "--token", "t"]),
            "TODO resource service requires --data-dir",
        ),
        (
            "resource service requires a data dir",
            strings(["--resources", "--token", "t"]),
            "resource service requires --data-dir",
        ),
        (
            "unreadable token file",
            strings(["--token-file", &path(&missing)]),
            "read token file",
        ),
        (
            "whitespace-only token file",
            strings(["--token-file", &path(&empty)]),
            "token file contains an empty bearer token",
        ),
        (
            "whitespace-only metrics token file",
            strings(["--metrics-token-file", &path(&metrics_empty), "--token", "t"]),
            "metrics token file contains an empty bearer token",
        ),
        (
            "metrics token must differ from server token",
            strings(["--token-file", &path(&token), "--metrics-token-file", &path(&metrics_same)]),
            "metrics token must differ from server token",
        ),
        (
            // `--web` takes an explicit boolean (Set action, like
            // --auth-enabled); a non-boolean is a usage error.
            "web flag rejects a non-boolean value",
            strings(["--web", "disable", "--token", "t"]),
            "invalid value 'disable' for '--web",
        ),
        (
            "no token flag, file or env",
            strings([]),
            "server token required: pass --token, --token-file, or set OPENCODER_SERVER_TOKEN",
        ),
    ];

    for (name, args, expected) in cases {
        let (success, stderr) = reject(&args);
        assert!(!success, "{name}: exited successfully with args {args:?}");
        assert!(
            stderr.contains(expected),
            "{name}: stderr misses {expected:?}\n--- stderr ---\n{stderr}"
        );
    }
}

/// Borrow `&str` args into owned `String`s (table rows own their paths).
fn strings<const N: usize>(args: [&str; N]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}
