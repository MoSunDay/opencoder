use super::support::*;
use opencoder_dag::{
    control::{Action, Command, Phase},
    DagRunStatus,
};
use opencoder_dag_runtime::{control, execute_run};
use serde_json::json;
use std::time::Duration;

async fn wait(f: &Fixture, run: &opencoder_dag::DagClaimedRun, mut check: impl FnMut() -> bool) {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "controlled step did not converge: states={:?}, events={:?}",
        control::status(&f.root, &run.run_id),
        f.events.0.lock().unwrap()
    );
}
fn command(action: Action, id: &str, step: &str, instance: Option<usize>, attempt: u64) -> Command {
    Command {
        operation_id: id.into(),
        step: step.into(),
        instance,
        expected_attempt: attempt,
        action,
        reason: "inspect original attempt".into(),
    }
}
const PROBE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
int main(int argc,char**argv){
 int first=open("started",O_CREAT|O_EXCL|O_WRONLY,0600);
 if(first>=0){close(first);if(argc>1&&!strcmp(argv[1],"slow"))sleep(120);
  if(argc>1&&!strcmp(argv[1],"fail"))return 9;}
 FILE *f=fopen("output.json","w");if(!f)return 2;fputs("{\"ok\":true}",f);fclose(f);
 return 0;
}
"#;

#[tokio::test]
async fn operator_agent_resume_keeps_session_and_does_not_repeat_original_input() {
    let f = Fixture::new().await;
    let run = f.run(json!([{"name":"process","recovery_policy":"operator","kind":{"type":"agent","prompt":"ORIGINAL-INPUT-ONCE"}}]), json!({"fixed":true}));
    let client = Scripted::new(|text| {
        if text.contains(opencoder_dag_runtime::exec::recovery::CONTINUE_PROMPT) {
            assert_eq!(text.matches("ORIGINAL-INPUT-ONCE").count(), 1);
            (Duration::ZERO, Ok("{\"recovered\":true}".into()))
        } else {
            (Duration::from_secs(120), Ok("{}".into()))
        }
    });
    let deps = f.deps(client.clone());
    let (cancel, rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(execute_run(deps, run.clone(), rx));
    wait(&f, &run, || !client.requests.lock().unwrap().is_empty()).await;
    let original = f.json(&run, "process/session.json")["session_id"].clone();
    control::submit(
        &f.root,
        &run.run_id,
        &run.spec,
        command(Action::Pause, "pause-agent", "process", None, 1),
    )
    .unwrap();
    wait(&f, &run, || {
        control::status(&f.root, &run.run_id)
            .unwrap()
            .iter()
            .any(|s| s.phase == Phase::Paused)
    })
    .await;
    control::submit(
        &f.root,
        &run.run_id,
        &run.spec,
        command(Action::Resume, "resume-agent", "process", None, 1),
    )
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        DagRunStatus::Done
    );
    drop(cancel);
    assert_eq!(f.json(&run, "process/session.json")["session_id"], original);
    assert_eq!(
        f.json(&run, "process/attempts/1/session.json")["session_id"],
        original
    );
    assert_eq!(
        f.json(&run, "process/output.json"),
        json!({"recovered":true})
    );
    assert_eq!(client.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn operator_pause_reaps_only_target_and_defers_finalizer_until_resume() {
    let f = Fixture::new().await;
    f.binary("controlled", PROBE);
    let run=f.run(json!([
        {"name":"process","recovery_policy":"operator","kind":{"type":"binary","resource":"controlled","args":["slow"]}},
        {"name":"independent","kind":{"type":"binary","resource":"controlled"}},
        {"name":"finalize","depends_on":["process","independent"],"trigger_rule":"all_done","kind":{"type":"binary","resource":"controlled"}}
    ]),json!({"fixed":true}));
    let root = f.root.clone();
    let id = run.run_id.clone();
    let deps = f.deps(Scripted::new(|_| panic!("binary test must not call model")));
    let (cancel, rx) = tokio::sync::watch::channel(false);
    let execution = run.clone();
    let task = tokio::spawn(async move {
        let result = execute_run(deps, execution, rx).await;
        if let Err(error) = &result {
            eprintln!("execution failed: {error:#}");
        }
        result
    });
    wait(&f, &run, || {
        root.join(&id).join("workspace/process/started").exists()
    })
    .await;
    control::submit(
        &root,
        &id,
        &run.spec,
        command(Action::Pause, "pause-work", "process", None, 1),
    )
    .unwrap();
    wait(&f, &run, || {
        control::status(&root, &id)
            .unwrap()
            .iter()
            .any(|s| s.step == "process" && s.phase == Phase::Paused)
    })
    .await;
    wait(&f, &run, || {
        root.join(&id).join("independent/meta.json").exists()
            && f.json(&run, "independent/meta.json")["outcome"] == "done"
    })
    .await;
    assert!(!root.join(&id).join("finalize/meta.json").exists());
    assert!(!task.is_finished());
    assert_eq!(
        control::receipt(&root, &id, "pause-work")
            .unwrap()
            .unwrap()
            .phase,
        "applied"
    );
    control::submit(
        &root,
        &id,
        &run.spec,
        command(Action::Resume, "resume-work", "process", None, 1),
    )
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        DagRunStatus::Done
    );
    drop(cancel);
    assert_eq!(f.json(&run, "process/meta.json")["outcome"], "done");
    assert_eq!(
        f.json(&run, "process/attempts/1/meta.json")["outcome"],
        "paused"
    );
    assert_eq!(
        control::status(&root, &id)
            .unwrap()
            .iter()
            .find(|s| s.step == "process")
            .unwrap()
            .attempt,
        2
    );
    assert_eq!(f.json(&run, "finalize/meta.json")["outcome"], "done");
}

#[tokio::test]
async fn operator_dynamic_failure_waits_without_cancelling_other_instances() {
    let f = Fixture::new().await;
    f.binary("controlled", PROBE);
    let run=f.run(json!([
        {"name":"process","recovery_policy":"operator","kind":{"type":"dynamic","failure_policy":"collect_all","source":{"type":"input","pointer":"/items"},"template":{"type":"binary","resource":"controlled"}}},
        {"name":"finalize","depends_on":["process"],"trigger_rule":"all_done","kind":{"type":"binary","resource":"controlled"}}
    ]),json!({"items":[["fail"],[]]}));
    let root = f.root.clone();
    let id = run.run_id.clone();
    let deps = f.deps(Scripted::new(|_| panic!("binary test must not call model")));
    let (cancel, rx) = tokio::sync::watch::channel(false);
    let execution = run.clone();
    let task = tokio::spawn(async move {
        let result = execute_run(deps, execution, rx).await;
        if let Err(error) = &result {
            eprintln!("execution failed: {error:#}");
        }
        result
    });
    wait(&f, &run, || {
        control::status(&root, &id)
            .unwrap()
            .iter()
            .any(|s| s.instance == Some(0) && s.phase == Phase::WaitingRecovery)
    })
    .await;
    wait(&f, &run, || {
        control::status(&root, &id)
            .unwrap()
            .iter()
            .any(|s| s.instance == Some(1) && s.phase == Phase::Done)
    })
    .await;
    assert!(!root.join(&id).join("finalize/meta.json").exists());
    control::submit(
        &root,
        &id,
        &run.spec,
        command(Action::Retry, "retry-instance", "process", Some(0), 1),
    )
    .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        DagRunStatus::Done
    );
    drop(cancel);
    let states = control::status(&root, &id).unwrap();
    assert_eq!(
        states
            .iter()
            .find(|s| s.instance == Some(0))
            .unwrap()
            .attempt,
        2
    );
    assert_eq!(
        states
            .iter()
            .find(|s| s.instance == Some(1))
            .unwrap()
            .attempt,
        1
    );
}
