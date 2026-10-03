#![cfg(windows)]
use opencoder_core::{
    harness::{CodexSettings, Harness},
    AgentKind, Config, Tool, ToolContext,
};
use opencoder_session::{
    process,
    tools::{
        self,
        command::{powershell, ShellTool},
    },
    SessionState,
};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[path = "windows_native/readonly.rs"]
mod readonly;
fn binary(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(name)
        .canonicalize()
        .unwrap()
}
fn configure() {
    process::configure_supervisor_binary(binary("mcp_mock_server.exe")).unwrap();
}
fn context(root: &Path) -> ToolContext {
    ToolContext {
        extra_env: vec![],
        session_id: "windows-test".into(),
        message_id: "message".into(),
        agent: "act".into(),
        working_dir: root.into(),
        max_output: 100_000,
        proxy: None,
        tools_path: None,
    }
}
async fn cleaned() {
    process::wait_for_owned_processes(tokio::time::Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    assert!(tools::bg::list().is_empty());
}

#[tokio::test]
async fn native_shell_handles_unicode_paths_streams_exit_and_tool_registration() {
    let _lock = SERIAL.lock().await;
    configure();
    let root = tempfile::tempdir().unwrap();
    let working = root.path().join("中文 空格");
    std::fs::create_dir(&working).unwrap();
    assert!(tools::registry().contains_key("powershell"));
    assert!(!tools::registry().contains_key("bash"));
    for agent in opencoder_core::agent::builtin_agents() {
        if agent.name == "plan" || agent.name == "sidecar" {
            assert!(agent.tools.allows("powershell"));
            assert!(!agent.tools.allows("bash"));
        }
    }
    let output = ShellTool.execute(json!({"command":"Set-Content -LiteralPath '中文 file.txt' -Value '内容'; [Console]::Out.WriteLine('中文输出'); [Console]::Error.WriteLine('错误输出')"}), &context(&working)).await.unwrap();
    assert!(!output.is_error, "{}", output.content);
    assert!(output.content.contains("中文输出") && output.content.contains("错误输出"));
    assert!(std::fs::read_to_string(working.join("中文 file.txt"))
        .unwrap()
        .contains("内容"));
    let failure = ShellTool
        .execute(json!({"command":"exit 7"}), &context(&working))
        .await
        .unwrap();
    assert!(failure.is_error && failure.content.contains("[exit code: 7]"));
    cleaned().await;
}

#[tokio::test]
async fn powershell_ast_gate_blocks_writes_without_evaluating_them() {
    let _lock = SERIAL.lock().await;
    configure();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("notes.txt"), "inspect").unwrap();
    for script in [
        "Get-Content notes.txt",
        "Get-ChildItem | Select-Object Name",
        "git status",
        "cat notes.txt",
    ] {
        powershell::read_only(script, root.path()).await.unwrap();
    }
    for script in [
        "Set-Content denied.txt x",
        "Get-Content notes.txt > denied.txt",
        "$(Set-Content denied.txt x)",
        "& { Set-Content denied.txt x }",
        "Invoke-Expression 'Set-Content denied.txt x'",
        "git diff --output=denied.txt",
        "Get-Content (",
    ] {
        let denial = opencoder_session::bash_guard::gate_async(
            &AgentKind::Plan,
            "plan",
            "powershell",
            Some(script),
            root.path(),
        )
        .await;
        assert!(denial.is_some(), "{script}");
        assert!(
            !root.path().join("denied.txt").exists(),
            "inspection executed {script}"
        );
    }
    cleaned().await;
}

#[tokio::test]
async fn cancelling_shell_and_natural_exit_stop_all_descendants() {
    let _lock = SERIAL.lock().await;
    configure();
    let root = tempfile::tempdir().unwrap();
    for suffix in ["; Start-Sleep 60", ""] {
        let script = format!("$p = Start-Process pwsh -ArgumentList '-NoLogo','-NoProfile','-NonInteractive','-Command','Start-Sleep 60' -PassThru -WindowStyle Hidden; Set-Content child.txt $p.Id{suffix}");
        let context = context(root.path());
        let task =
            tokio::spawn(
                async move { ShellTool.execute(json!({"command":script}), &context).await },
            );
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(root.path().join("child.txt")) {
                    if let Ok(pid) = value.trim().parse::<u32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if suffix.is_empty() {
            assert!(!task.await.unwrap().unwrap().is_error);
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        if !handle.is_null() {
            assert_eq!(
                unsafe { WaitForSingleObject(handle, 10_000) },
                0,
                "descendant survived"
            );
            unsafe {
                CloseHandle(handle);
            }
        }
        cleaned().await;
        std::fs::remove_file(root.path().join("child.txt")).unwrap();
    }
}

#[tokio::test]
async fn native_codex_streams_and_resumes_the_persisted_thread() {
    let _lock = SERIAL.lock().await;
    configure();
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(root.path().join("config-home"));
    use opencoder_store::{LibsqlStore, Store};
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let config = Config::default();
    let mut session = SessionState::new(
        "windows-codex",
        opencoder_core::resolve_agent("act").unwrap(),
        config.clone(),
        opencoder_session::harness::configured_client(config.clone()),
        root.path().into(),
    )
    .with_store(store.clone());
    session.harness.harness = Harness::Codex;
    session.harness.codex = Some(CodexSettings {
        executable: Some(binary("mcp_mock_server.exe").to_string_lossy().into_owned()),
        ..Default::default()
    });
    session.harness.envs.insert(
        "CAPTURE".into(),
        root.path()
            .join("capture.jsonl")
            .to_string_lossy()
            .into_owned(),
    );
    opencoder_session::run(&mut session, "原生需求".into(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        store
            .harness_runtime(&session.id)
            .await
            .unwrap()
            .unwrap()
            .thread_id
            .as_deref(),
        Some("windows-fixture-thread")
    );
    let mut resumed = opencoder_session::resume(
        store,
        &session.id,
        config,
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    opencoder_session::run(&mut resumed, "follow up".into(), |_| {})
        .await
        .unwrap();
    let records: Vec<serde_json::Value> =
        std::fs::read_to_string(root.path().join("capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(records.len(), 2);
    assert!(records[0]["prompt"].as_str().unwrap().contains("原生需求"));
    assert_eq!(records[1]["args"][1], "resume");
    assert_eq!(records[1]["args"][2], "windows-fixture-thread");
    cleaned().await;
}

#[tokio::test]
async fn killing_owner_process_closes_job_and_kills_grandchildren() {
    let _lock = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("child.txt");
    let mut parent = std::process::Command::new(binary("mcp_mock_server.exe"))
        .arg("--job-parent")
        .arg(&path)
        .spawn()
        .unwrap();
    let pid = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(&path) {
                if let Ok(pid) = value.parse::<u32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!handle.is_null());
    parent.kill().unwrap();
    parent.wait().unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(handle, 10_000) },
        0,
        "grandchild survived parent death"
    );
    unsafe {
        CloseHandle(handle);
    }
}

#[tokio::test]
async fn native_search_visits_junction_targets_once() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("file.txt"), "unique match\n").unwrap();
    for name in ["first", "second"] {
        assert!(std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(root.path().join(name))
            .arg(&real)
            .status()
            .unwrap()
            .success());
    }
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tools::search::SearchTool.execute(json!({"pattern":"unique match"}), &context(root.path())),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.is_error, "{}", output.content);
    assert_eq!(
        output.content.matches("unique match").count(),
        1,
        "{}",
        output.content
    );
}
