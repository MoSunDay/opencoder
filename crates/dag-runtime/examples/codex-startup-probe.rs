//! Actual runc acceptance for managed startup/private mounts/host leases.
//! The fixture uses deterministic quota and a process probe, never credentials.
use anyhow::{ensure, Context, Result};
use opencoder_core::harness::{CodexSettings, Versioned};
use opencoder_dag_runtime::sandbox::run::{execute, RunContainer, StepProcess};
use serde_json::json;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: codex-startup-probe FIXTURE ROOTFS READONLY_NFS_SOURCE"
    );
    let fixture = PathBuf::from(&args[0]);
    let rootfs = PathBuf::from(&args[1]);
    let source = PathBuf::from(&args[2]);
    let startup: Vec<String> =
        serde_json::from_slice(&std::fs::read(fixture.join("startup-command.json"))?)?;
    let agents = fixture.join("agents");
    std::fs::create_dir_all(agents.join("probe"))?;
    std::fs::write(
        agents.join("probe/meta.json"),
        json!({"name":"probe","harness":"codex","harness_profile":"probe"}).to_string(),
    )?;
    let mut config = opencoder_core::Config::default();
    config.agent.agents_dir = Some(agents);
    config.dag.rootfs_dir = Some(rootfs);
    config.dag.workspace_dir = Some(source);
    config.agent.runtime.profiles.insert(
        "probe".into(),
        Versioned {
            revision: 1,
            settings: CodexSettings {
                startup_script: startup.clone(),
                executable: Some("/usr/bin/fixture-codex".into()),
                envs: [(
                    "CODEX_HOME".into(),
                    fixture.join("home").display().to_string(),
                )]
                .into(),
                ..Default::default()
            },
        },
    );
    let run = serde_json::from_value(
        json!({"run_id":"codex-startup-probe", "dag_id":"codex-startup-probe", "created_at":0,
        "spec":{"name":"codex-startup-probe","steps":[{"name":"first","kind":{"type":"agent","agent":"probe","prompt":"fixture"}}]}}),
    )?;
    let root = fixture.join("container");
    let container = RunContainer::start(&root, &config, &run).await?;
    let result = probe(&root, &startup, &fixture).await;
    container.cleanup().await?;
    let value = result?;
    std::fs::write(
        fixture.join("result.json"),
        serde_json::to_vec_pretty(&value)?,
    )?;
    println!("{}", value);
    Ok(())
}

fn process(startup: &[String], key: &str, sleep: &str) -> StepProcess {
    let mut argv = startup.to_vec();
    argv.extend(["codex".into(), "exec".into(), "--json".into(), "-".into()]);
    StepProcess { key:key.into(),argv, cwd:"/workspace/first".into(),timeout_secs:Some(30),env:vec![
        ("OPENCODER_CODEX_LAUNCH".into(),"/run/opencoder-private/codex/first/launch.json".into()),
        ("OPENCODER_STEP_SESSION_ID".into(),key.into()),
        ("OPENCODER_HARNESS_CONTEXT".into(),json!({"schema_version":1,"session_id":key,"thread_id":"same-thread","input_id":format!("input-{key}")}).to_string()),
        ("FIXTURE_SLEEP".into(),sleep.into())] }
}

async fn probe(
    root: &std::path::Path,
    startup: &[String],
    fixture: &std::path::Path,
) -> Result<serde_json::Value> {
    let (first, second) = tokio::join!(
        execute(
            root,
            process(startup, "first-turn", ".4"),
            CancellationToken::new(),
            None
        ),
        execute(
            root,
            process(startup, "second-turn", ".4"),
            CancellationToken::new(),
            None
        )
    );
    let outputs = [first?, second?];
    ensure!(
        outputs.iter().all(|(code, _)| *code == 7),
        "probe child exit status was not preserved"
    );
    let values: Vec<serde_json::Value> = outputs
        .iter()
        .map(|(_, text)| serde_json::from_str(text.trim()))
        .collect::<Result<_, _>>()?;
    let mut slots: Vec<_> = values
        .iter()
        .map(|value| value["args"][1].as_str().unwrap().to_owned())
        .collect();
    slots.sort();
    ensure!(
        slots == ["1", "2"],
        "host quota ledger did not spread parallel runc slots"
    );
    ensure!(
        values.iter().all(|v| v["broker"]
            .as_str()
            .unwrap_or("")
            .starts_with("/run/opencoder-private/exec/")),
        "wrapper did not use its private socket"
    );
    ensure!(
        values
            .iter()
            .all(|v| v["connection_files_readable"] == true),
        "declared connection or its exact private token file was not mounted"
    );
    let cancel = CancellationToken::new();
    let running = execute(
        root,
        process(startup, "cancel-turn", "30"),
        cancel.clone(),
        None,
    );
    let cancelling = async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        cancel.cancel();
    };
    let (cancelled, _) = tokio::join!(running, cancelling);
    ensure!(cancelled.is_err(), "cancelled probe unexpectedly succeeded");
    let ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.join("runners/codex/leases.json"))?)?;
    ensure!(
        ledger
            .as_object()
            .context("lease ledger is not an object")?
            .is_empty(),
        "runc account reservation leaked after cleanup"
    );
    let receipts: Vec<serde_json::Value> =
        std::fs::read_dir(fixture.join("runners/codex/receipts"))?
            .map(|entry| Ok(serde_json::from_slice(&std::fs::read(entry?.path())?)?))
            .collect::<Result<_>>()?;
    ensure!(
        receipts.len() == 3
            && receipts
                .iter()
                .all(|r| r["state"] == "released" && r["thread_id"] == "same-thread"),
        "session/thread receipt or release differs"
    );
    Ok(
        json!({"status":"PASS","real_runc":true,"parallel_slots":slots,"cancelled":true,"leases_empty":true,
        "session_receipts":receipts.len(),"outputs":values}),
    )
}
