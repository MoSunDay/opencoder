#![cfg(unix)]
#[path = "harness/session.rs"]
mod fixtures;
use opencoder_core::{harness::CodexSettings, Config};
use opencoder_session::run;
use serde_json::Value;
use std::path::Path;

fn script(root: &Path, text: &str) -> Vec<String> {
    let path = root.join("startup script.sh");
    std::fs::write(&path, text).unwrap();
    vec!["/bin/sh".into(), path.display().to_string()]
}

fn records(root: &Path, name: &str) -> Vec<Value> {
    std::fs::read_to_string(root.join(name))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn external_launcher_sets_environment_and_wraps_each_pinned_resume_and_fork() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = fixtures::session(root.path()).await;
    let mut command = script(
        root.path(),
        r#"set -eu
test "$1" = 'literal $(argument) 中文'
shift
printf '%s\n' "$OPENCODER_HARNESS_CONTEXT" >> context.jsonl
export EXAMPLE='由脚本设置 = $(literal)'
export PATH="$PWD/bin:/usr/bin:/bin"
"$@"
printf 'finished\n' >> cleanup.txt
"#,
    );
    command.push("literal $(argument) 中文".into());
    session
        .harness
        .envs
        .insert("PATH".into(), "/missing-binaries".into());
    session.config.agent.codex = Some(CodexSettings {
        startup_script: command.clone(),
        model: Some("model with spaces".into()),
        ..Default::default()
    });
    run(&mut session, "需求 $(literal)".into(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        session.harness.codex.as_ref().unwrap().startup_script,
        command
    );
    let mut changed = Config::default();
    changed.agent.codex = Some(CodexSettings {
        startup_script: vec!["/does/not/exist".into()],
        ..Default::default()
    });
    let mut resumed = opencoder_session::resume(
        store.clone(),
        &session.id,
        changed,
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    run(&mut resumed, "follow up".into(), |_| {}).await.unwrap();
    let fork = opencoder_session::fork::fork_session(store.as_ref(), &session.id)
        .await
        .unwrap();
    let mut forked = opencoder_session::resume(
        store.clone(),
        &fork,
        Config::default(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    run(&mut forked, "fork".into(), |_| {}).await.unwrap();
    let calls = records(root.path(), "capture.jsonl");
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|r| r["env"] == "由脚本设置 = $(literal)"));
    assert!(calls[0]["prompt"]
        .as_str()
        .unwrap()
        .ends_with("需求 $(literal)"));
    assert_eq!(
        calls[0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| *x == "model with spaces")
            .count(),
        1
    );
    assert_eq!(calls[1]["args"][1], "resume");
    assert_eq!(calls[2]["args"][1], "fork");
    let contexts = records(root.path(), "context.jsonl");
    assert_eq!(contexts[0]["schema_version"], 1);
    assert_eq!(contexts[0]["session_id"], session.id);
    assert!(contexts[0]["thread_id"].is_null());
    assert_eq!(contexts[1]["thread_id"], "fixture-thread");
    assert_eq!(contexts[2]["fork_from"], "fixture-thread");
    assert_ne!(contexts[0]["input_id"], contexts[1]["input_id"]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("cleanup.txt"))
            .unwrap()
            .lines()
            .count(),
        3
    );
    assert!(!root.path().join("argument").exists());
    assert!(
        !store
            .harness_runtime(&session.id)
            .await
            .unwrap()
            .unwrap()
            .envs
            .values()
            .any(|v| v.contains("由脚本设置")),
        "script-created values must not be persisted"
    );
}

#[tokio::test]
async fn failed_or_missing_startup_script_never_falls_back_to_codex() {
    for missing in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (mut session, _) = fixtures::session(root.path()).await;
        session.config.agent.codex = Some(CodexSettings {
            startup_script: if missing {
                vec!["/missing-launcher".into()]
            } else {
                script(root.path(), "exit 17\n")
            },
            ..Default::default()
        });
        let error = run(&mut session, "test".into(), |_| {}).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains(if missing { "startup script" } else { "17" }),
            "{error:#}"
        );
        assert!(!root.path().join("capture.jsonl").exists());
        assert!(!session.harness.in_flight);
    }
}

#[tokio::test]
async fn startup_cancellation_reaps_script_and_descendant_before_codex_launch() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, _) = fixtures::session(root.path()).await;
    session.config.agent.codex = Some(CodexSettings {
        startup_script: script(
            root.path(),
            "sleep 120 &\nprintf '%s' \"$!\" > startup.pid\nwait\nexec \"$@\"\n",
        ),
        ..Default::default()
    });
    let token = tokio_util::sync::CancellationToken::new();
    session.cancel = Some(token.clone());
    let marker = root.path().join("startup.pid");
    let cancellation = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(pid) = tokio::fs::read_to_string(&marker).await {
                    if pid.parse::<u32>().is_ok() {
                        token.cancel();
                        return pid;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        run(&mut session, "test".into(), |_| {}),
    )
    .await
    .unwrap()
    .unwrap();
    let pid = cancellation.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let output = tokio::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid])
                .output()
                .await
                .unwrap();
            let state = String::from_utf8(output.stdout).unwrap();
            if state.trim().is_empty() || state.trim().starts_with('Z') {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!root.path().join("capture.jsonl").exists());
    assert!(!session.harness.in_flight);
}
