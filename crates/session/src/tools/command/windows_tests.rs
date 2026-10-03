use super::*;
use serde_json::json;

#[tokio::test]
async fn powershell_timeout_handoff_completes_and_output_overflow_stops_background() {
    let _lock = super::super::bg::test_registry_mutex().lock().await;
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("mcp_mock_server.exe");
    crate::process::configure_supervisor_binary(binary).unwrap();
    let root = tempfile::tempdir().unwrap();
    let context = ToolContext {
        extra_env: vec![],
        session_id: "windows-handoff".into(),
        message_id: "message".into(),
        agent: "act".into(),
        working_dir: root.path().into(),
        max_output: 100_000,
        proxy: None,
        tools_path: None,
    };
    let output = ShellTool
        .execute(
            json!({"command":"Write-Output before; Start-Sleep 3; Write-Output after"}),
            &context,
        )
        .await
        .unwrap();
    assert!(
        !output.is_error && output.content.contains("[powershell-timeout:"),
        "{}",
        output.content
    );
    let pid = super::super::bg::list()[0].pid;
    let path = output_path(pid);
    assert!(opencoder_core::platform::fs::private_access(&path).unwrap());
    tokio::time::timeout(Duration::from_secs(20), async {
        while !super::super::bg::list().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let output = std::fs::read_to_string(path).unwrap();
    assert!(output.contains("before") && output.contains("after"));
    let output = ShellTool.execute(json!({"command":"Start-Sleep 2; [Console]::Out.Write(('x' * 9000000)); Start-Sleep 60"}), &context).await.unwrap();
    assert!(
        output.content.contains("[powershell-timeout:"),
        "{}",
        output.content
    );
    tokio::time::timeout(Duration::from_secs(20), async {
        while !super::super::bg::list().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    super::super::bg::cleanup_all();
    crate::process::wait_for_owned_processes(tokio::time::Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
}
