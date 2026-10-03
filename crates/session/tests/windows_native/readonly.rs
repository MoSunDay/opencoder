use super::*;

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "0")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn prepared_read_only_commands_disable_git_and_rg_external_configuration() {
    let _lock = SERIAL.lock().await;
    configure();
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init"]);
    git(
        root.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "Native test"]);
    std::fs::write(root.path().join(".gitattributes"), "*.txt diff=hostile\n").unwrap();
    std::fs::write(root.path().join("notes.txt"), "before\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture"]);
    std::fs::write(root.path().join("notes.txt"), "after 中文\n").unwrap();
    let helper = "pwsh -NoProfile -Command \"Set-Content marker.txt unsafe\"";
    for key in ["diff.hostile.textconv", "diff.external", "core.fsmonitor"] {
        git(root.path(), &["config", key, helper]);
    }
    let index_before = std::fs::read(root.path().join(".git/index")).unwrap();
    let mut ctx = context(root.path());
    ctx.extra_env = vec![
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), "core.fsmonitor".into()),
        ("GIT_CONFIG_VALUE_0".into(), helper.into()),
        ("GIT_EXTERNAL_DIFF".into(), helper.into()),
        (
            "GIT_TRACE".into(),
            root.path().join("trace.txt").display().to_string(),
        ),
        (
            "GIT_REDIRECT_STDOUT".into(),
            root.path().join("redirect.txt").display().to_string(),
        ),
    ];
    for script in [
        "git diff",
        "git status --short",
        "git log -n1 --oneline",
        "git log '--format=%s \"%an\"'",
        "git show",
        "git grep 'after'",
        "Get-Content notes.txt | Select-Object -First 1",
    ] {
        let prepared = powershell::prepare_read_only(script, root.path())
            .await
            .unwrap();
        let output = ShellTool
            .execute(json!({"command":prepared}), &ctx)
            .await
            .unwrap();
        assert!(!output.is_error, "{script}: {}", output.content);
    }
    // Exercise dispatch as well as the prepared tool: the runner must execute
    // the controlled script for both agents, never the originally checked Git.
    for agent in ["plan", "sidecar"] {
        use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient};
        let client = Arc::new(
            MockChatClient::new()
                .push_script(vec![LlmEvent::Completed {
                    text: String::new(),
                    tool_calls: vec![CompletedToolCall {
                        id: "readonly-git".into(),
                        name: "powershell".into(),
                        input: json!({"command":"git diff"}),
                    }],
                    usage: None,
                }])
                .push_script(vec![LlmEvent::Completed {
                    text: "done".into(),
                    tool_calls: vec![],
                    usage: None,
                }]),
        );
        let mut session = SessionState::new(
            format!("readonly-{agent}"),
            opencoder_core::resolve_agent(agent).unwrap(),
            Config {
                model: "mock/test".into(),
                ..Config::default()
            },
            client,
            root.path().into(),
        );
        session.env_passthrough = ctx.extra_env.clone();
        let mut completed = Vec::new();
        opencoder_session::run(&mut session, "inspect changes".into(), |event| {
            if let opencoder_session::SessionEvent::ToolEnd {
                is_error, output, ..
            } = event
            {
                completed.push((is_error, output));
            }
        })
        .await
        .unwrap();
        assert_eq!(completed.len(), 1, "{agent}");
        assert!(
            !completed[0].0 && completed[0].1.contains("after"),
            "{agent}: {completed:?}"
        );
    }
    assert_eq!(
        std::fs::read(root.path().join(".git/index")).unwrap(),
        index_before
    );
    for file in ["marker.txt", "trace.txt", "redirect.txt"] {
        assert!(!root.path().join(file).exists());
    }
    git(root.path(), &["config", "filter.hostile.clean", helper]);
    let prepared = powershell::prepare_read_only("git status", root.path())
        .await
        .unwrap();
    assert!(
        ShellTool
            .execute(json!({"command":prepared}), &ctx)
            .await
            .unwrap()
            .is_error
    );
    assert!(!root.path().join("marker.txt").exists());
    std::fs::write(root.path().join("rg-config"), format!("--pre\n{helper}\n")).unwrap();
    ctx.extra_env.push((
        "RIPGREP_CONFIG_PATH".into(),
        root.path().join("rg-config").display().to_string(),
    ));
    let prepared = powershell::prepare_read_only("rg 'after' notes.txt", root.path())
        .await
        .unwrap();
    let output = ShellTool
        .execute(json!({"command":prepared}), &ctx)
        .await
        .unwrap();
    assert!(
        !output.is_error && output.content.contains("after"),
        "{}",
        output.content
    );
    assert!(!root.path().join("marker.txt").exists());
    cleaned().await;
}
