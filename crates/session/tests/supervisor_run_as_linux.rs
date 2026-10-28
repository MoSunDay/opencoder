//! Real supervisor round-trip for privilege dropping: the bash tool spawns
//! below the hidden Linux supervisor carrying `--uid/--gid/--groups`; the
//! supervisor drops the workload to a provisioned system user and the command
//! really runs as that user. Requires root (useradd/userdel); skipped
//! elsewhere.
#![cfg(target_os = "linux")]

use opencoder_core::platform::RunAsIdentity;
use opencoder_core::{Tool, ToolContext};
use opencoder_session::process::configure_supervisor_binary;
use opencoder_session::tools::bash::BashTool;
use std::os::unix::fs::chown;
use std::path::Path;
use std::process::Command;

fn root_only() -> bool {
    let running_as_root = unsafe { libc::geteuid() } == 0;
    if !running_as_root {
        eprintln!("skipping: supervisor run_as tests require root");
    }
    running_as_root
}

/// Provision a real throwaway system user and return its captured ids.
fn provision(username: &str, home: &Path) -> (u32, u32) {
    let output = Command::new("useradd")
        .args([
            "-r",
            "-M",
            "-d",
            &home.to_string_lossy(),
            "-s",
            "/usr/sbin/nologin",
            username,
        ])
        .output()
        .expect("useradd runs");
    assert!(
        output.status.success(),
        "useradd failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let uid = capture_id("-u", username);
    let gid = capture_id("-g", username);
    (uid, gid)
}

fn capture_id(flag: &str, username: &str) -> u32 {
    let output = Command::new("id")
        .arg(flag)
        .arg(username)
        .output()
        .expect("id runs");
    assert!(
        output.status.success(),
        "id {} {} failed: {}",
        flag,
        username,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("numeric id")
}

/// Delete the account even when an assertion failed mid-test.
struct AccountGuard(String);
impl Drop for AccountGuard {
    fn drop(&mut self) {
        let _ = Command::new("userdel").arg(&self.0).output();
    }
}

fn context(workdir: &Path, session_id: &str) -> ToolContext {
    ToolContext {
        session_id: session_id.into(),
        message_id: "run-as-message".into(),
        agent: "act".into(),
        working_dir: workdir.to_path_buf(),
        max_output: 100_000,
        proxy: None,
        tools_path: None,
        // The login shell must resolve its profile inside the dropped user's
        // own home, not the inherited node home it can no longer read.
        extra_env: vec![("HOME".into(), workdir.display().to_string())],
    }
}

#[tokio::test]
async fn supervisor_runs_the_workload_as_the_provisioned_user() {
    if !root_only() {
        return;
    }
    let suffix = ulid::Ulid::new().to_string().to_lowercase();
    // Linux user names cap at 32 bytes; 12 chars keep the prefix below it.
    let username = format!("ocop-sup-{}", &suffix[..12]);
    let guard = AccountGuard(username.clone());
    let home = tempfile::tempdir().unwrap();
    let (uid, gid) = provision(&username, home.path());
    chown(home.path(), Some(uid), Some(gid)).unwrap();

    let supervisor = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mcp_mock_server"));
    configure_supervisor_binary(supervisor).unwrap();
    let tool = BashTool::new(Some(RunAsIdentity {
        uid,
        gid,
        groups: Vec::new(),
        username: Some(username.clone()),
    }));
    let ctx = context(home.path(), &format!("run-as-{suffix}"));

    let out = tool
        .execute(
            serde_json::json!({"command": "echo uid=$(id -u) gid=$(id -g) umask=$(umask)"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!out.is_error, "identity probe failed: {}", out.content);
    assert_eq!(
        out.content.trim(),
        format!("uid={uid} gid={gid} umask=0077"),
        "workload did not run as the provisioned user"
    );

    let deny = format!("/root/deny-{suffix}");
    let out = tool
        .execute(serde_json::json!({"command": format!("touch {deny}")}), &ctx)
        .await
        .unwrap();
    assert!(
        out.is_error && out.content.contains("exit code: 1"),
        "write outside home must fail: {}",
        out.content
    );
    assert!(!Path::new(&deny).exists());
    drop(guard);
}

#[tokio::test]
async fn supervisor_without_identity_keeps_the_node_user() {
    if !root_only() {
        return;
    }
    let workdir = tempfile::tempdir().unwrap();
    let supervisor = std::path::PathBuf::from(env!("CARGO_BIN_EXE_mcp_mock_server"));
    configure_supervisor_binary(supervisor).unwrap();
    let tool = BashTool::new(None);
    let ctx = context(workdir.path(), "run-as-identity-absent");
    let out = tool
        .execute(serde_json::json!({"command": "id -u"}), &ctx)
        .await
        .unwrap();
    assert!(!out.is_error, "plain probe failed: {}", out.content);
    assert_eq!(out.content.trim(), "0", "no identity means the node user");
}
