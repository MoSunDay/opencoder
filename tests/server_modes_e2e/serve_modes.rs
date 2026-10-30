//! The standalone service modes and the startup seed:
//! - `--resources` / `--todo-resources` bind loopback SILENTLY (no
//!   `listening on` stdout line, host hard-coded) and answer
//!   `{"ok":true,"role":...,"build":...}` on a bearer-guarded health
//!   endpoint;
//! - a boot with `<workdir>/.opencoder/schedules.json` imports its jobs
//!   into the schedules table exactly once (a second boot against the
//!   same workdir must not duplicate them).

use std::path::Path;
use std::process::Command;

use serde_json::json;

use crate::proc::{free_port, wait_health, ServerProc};
use crate::support::http_util::http;

const TOKEN: &str = "resource-e2e-token";

/// Boot one silent resource service on a pre-picked port (the mode binds
/// `127.0.0.1` hard-coded and prints nothing, so readiness is a health
/// probe) and return `(proc, base_url)`.
fn boot_service(workdir: &Path, mode: &str, data: &Path) -> (ServerProc, String) {
    let port = free_port();
    let port_arg = port.to_string();
    let args = [
        mode,
        "--workdir",
        workdir.to_str().unwrap(),
        "--port",
        &port_arg,
        "--token",
        TOKEN,
        "--data-dir",
        data.to_str().unwrap(),
    ];
    let proc = ServerProc::spawn(workdir, &args, &[]);
    let base = format!("http://127.0.0.1:{port}");
    wait_health(proc.log(), &base, TOKEN, 30);
    (proc, base)
}

fn assert_health_role(base: &str, role: &str) {
    let (status, body) = http(base, "GET", "/api/health", TOKEN, "");
    assert_eq!(status, 200, "bearer health probe: {body}");
    assert_eq!(body["ok"], json!(true), "health body: {body}");
    assert_eq!(body["role"], json!(role), "health role: {body}");
    assert!(
        body["build"].is_object(),
        "health must embed build metadata: {body}"
    );
    let (status, _) = http(base, "GET", "/api/health", "", "");
    assert_eq!(status, 401, "health without Bearer must be refused");
}

#[test]
fn resources_service_serves_loopback_health() {
    let tmp = tempfile::tempdir().unwrap();
    let (proc, base) = boot_service(tmp.path(), "--resources", &tmp.path().join("data"));
    assert_health_role(&base, "resources");
    // Silent mode: the resource service never prints the readiness line.
    assert!(
        !proc.log().contains("listening on"),
        "resource service must stay silent on stdout:\n{}",
        proc.log().tail(20)
    );
}

#[test]
fn todo_resources_service_serves_loopback_health() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    // Pinned reality: the TODO service's storage binding step needs the
    // data directory to exist; it does not create it on boot.
    std::fs::create_dir_all(&data).unwrap();
    let (proc, base) = boot_service(tmp.path(), "--todo-resources", &data);
    assert_health_role(&base, "todo-resources");
    assert!(
        !proc.log().contains("listening on"),
        "todo resource service must stay silent on stdout:\n{}",
        proc.log().tail(20)
    );
}

#[test]
fn startup_seed_imports_schedules_json_once() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().join(".opencoder");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("schedules.json"),
        json!({
            "schedules": [{
                "id": "seed-smoke",
                "cron": "*/5 * * * * *",
                "kind": "agent",
                "target": "act",
                "params": {"prompt": "seeded prompt"}
            }],
            "scan_interval_secs": 60
        })
        .to_string(),
    )
    .unwrap();

    let workdir = tmp.path().to_str().unwrap().to_string();
    let data_dir = tmp.path().join("data").to_str().unwrap().to_string();
    let boot = || {
        let args = [
            "--workdir",
            &workdir,
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--token",
            TOKEN,
            "--data-dir",
            &data_dir,
        ];
        let proc = ServerProc::spawn(tmp.path(), &args, &[]);
        let base = proc.wait_http_ready(60);
        (proc, base)
    };

    // First boot: the table is empty, so the file seeds it.
    let (first, base) = boot();
    let (status, body) = http(&base, "GET", "/api/schedules", TOKEN, "");
    assert_eq!(status, 200, "schedules listing: {body}");
    let rows = body["schedules"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1, "exactly one seeded schedule: {body}");
    assert_eq!(rows[0]["id"], json!("seed-smoke"));
    assert_eq!(rows[0]["kind"], json!("agent"));
    assert_eq!(rows[0]["params"]["prompt"], json!("seeded prompt"));
    // Stop the first boot with SIGTERM: the platform server retires
    // gracefully (exit 0) and releases the store for the second boot.
    let signalled = Command::new("kill")
        .args(["-TERM", &first.pid().to_string()])
        .status()
        .expect("send SIGTERM to the first boot");
    assert!(signalled.success());
    let status = first.wait_exit(30);
    assert_eq!(
        status.code(),
        Some(0),
        "platform server must exit 0 on SIGTERM"
    );
    drop(first);

    // Second boot on the SAME workdir: the table is no longer empty, so
    // the import must not run again (one-time seed, no duplicates).
    let (_second, base) = boot();
    let (status, body) = http(&base, "GET", "/api/schedules", TOKEN, "");
    assert_eq!(status, 200, "second boot schedules listing: {body}");
    let rows = body["schedules"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        rows.len(),
        1,
        "restart must not duplicate the seeded schedule: {body}"
    );
    assert_eq!(rows[0]["id"], json!("seed-smoke"));
}
