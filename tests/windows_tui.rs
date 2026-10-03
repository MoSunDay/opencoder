#![cfg(windows)]

#[tokio::test]
async fn windows_tui_short_commands_use_powershell_and_clean_timeout() {
    let executable = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("opencoder.exe");
    opencoder_session::process::configure_supervisor_binary(executable).unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = opencoder_tui::bash_exec::run_command(
        "[Console]::Out.WriteLine('中文'); [Console]::Error.WriteLine('stderr')",
        root.path(),
    )
    .await;
    assert!(
        output.contains("中文") && output.contains("stderr"),
        "{output}"
    );
    assert_eq!(
        opencoder_tui::bash_exec::run_command("$null", root.path()).await,
        "(no output)"
    );
    assert!(
        opencoder_tui::bash_exec::run_command("Start-Sleep 30", root.path())
            .await
            .starts_with("timeout")
    );
    opencoder_session::process::wait_for_owned_processes(
        tokio::time::Instant::now() + std::time::Duration::from_secs(10),
    )
    .await
    .unwrap();
}

#[test]
fn windows_main_starts_and_reports_product_metadata() {
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("opencoder.exe");
    let output = std::process::Command::new(binary)
        .arg("--build-info")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(info["git_commit"]
        .as_str()
        .is_some_and(|commit| commit.len() == 40));
}
