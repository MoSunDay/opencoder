use super::*;
use opencoder_dag::control::Command;
use serde_json::json;

fn run() -> DagClaimedRun {
    DagClaimedRun{run_id:"controlled-run".into(),dag_id:"controlled".into(),created_at:1,
        spec:serde_json::from_value(json!({"name":"controlled","steps":[{"name":"process","recovery_policy":"operator","kind":{"type":"binary","resource":"fixture"}}]})).unwrap()}
}
fn command(action: Action, id: &str, attempt: u64) -> Command {
    Command {
        operation_id: id.into(),
        step: "process".into(),
        instance: None,
        expected_attempt: attempt,
        action,
        reason: "inspect original state".into(),
    }
}
fn done(outcome: StepOutcome) -> StepDone {
    StepDone {
        name: "process".into(),
        instance: None,
        started_at_ms: 1,
        result: StepResult {
            outcome,
            error: Some("original failure".into()),
            output_text: "original trace".into(),
            output_json: None,
            session_id: None,
        },
    }
}

#[tokio::test]
async fn failure_waits_and_retry_preserves_history_and_attempt_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.begin("process", None).unwrap();
    assert!(managed
        .completed(&mut done(StepOutcome::Error), false)
        .await
        .unwrap());
    assert!(managed.waiting());
    assert!(!managed.ready("process", None));
    let c = command(Action::Retry, "retry-original", 1);
    let receipt = control::submit(tmp.path(), &run.run_id, &run.spec, c.clone()).unwrap();
    assert_eq!(receipt.phase, "queued");
    assert_eq!(
        control::submit(tmp.path(), &run.run_id, &run.spec, c.clone())
            .unwrap()
            .phase,
        "queued"
    );
    assert!(managed.poll(&BTreeMap::new(), false).unwrap().is_empty());
    assert!(managed.ready("process", None));
    managed.begin("process", None).unwrap();
    let state = control::status(tmp.path(), &run.run_id).unwrap();
    assert_eq!(state[0].attempt, 2);
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "retry-original")
            .unwrap()
            .unwrap()
            .phase,
        "applied"
    );
    let archived = tmp
        .path()
        .join(&run.run_id)
        .join("process/attempts/1/output.txt");
    assert_eq!(std::fs::read_to_string(archived).unwrap(), "original trace");
    assert!(control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Pause, "stale", 1)
    )
    .is_err());
    let mut different = c;
    different.reason = "changed".into();
    assert!(control::submit(tmp.path(), &run.run_id, &run.spec, different).is_err());
}

#[tokio::test]
async fn pause_receipt_waits_for_executor_cleanup_and_only_stops_target() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.begin("process", None).unwrap();
    let target = CancellationToken::new();
    let other = CancellationToken::new();
    let active = BTreeMap::from([
        (("process".into(), None), target.clone()),
        (("other".into(), None), other.clone()),
    ]);
    control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Pause, "pause-original", 1),
    )
    .unwrap();
    managed.poll(&active, false).unwrap();
    assert!(target.is_cancelled());
    assert!(!other.is_cancelled());
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "pause-original")
            .unwrap()
            .unwrap()
            .phase,
        "queued"
    );
    assert!(managed
        .completed(&mut done(StepOutcome::Cancelled), false)
        .await
        .unwrap());
    assert_eq!(
        control::status(tmp.path(), &run.run_id).unwrap()[0].phase,
        Phase::Paused
    );
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "pause-original")
            .unwrap()
            .unwrap()
            .phase,
        "applied"
    );
}

#[tokio::test]
async fn completed_attempt_cannot_be_reopened_and_global_cancel_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.begin("process", None).unwrap();
    assert!(!managed
        .completed(&mut done(StepOutcome::Done), false)
        .await
        .unwrap());
    control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Retry, "retry-done", 1),
    )
    .unwrap();
    assert!(managed.poll(&BTreeMap::new(), false).unwrap().is_empty());
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "retry-done")
            .unwrap()
            .unwrap()
            .phase,
        "rejected"
    );
    assert_eq!(
        control::status(tmp.path(), &run.run_id).unwrap()[0].phase,
        Phase::Done
    );
    control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Pause, "cancel-race", 1),
    )
    .unwrap();
    managed.poll(&BTreeMap::new(), true).unwrap();
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "cancel-race")
            .unwrap()
            .unwrap()
            .error
            .as_deref(),
        Some("run cancellation takes precedence")
    );
}

#[test]
fn restart_retains_unresolved_attempt_without_automatic_execution() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.begin("process", None).unwrap();
    drop(managed);
    let managed = Managed::open(tmp.path(), &run, true).unwrap();
    assert!(managed.waiting());
    assert!(!managed.ready("process", None));
    let status = control::status(tmp.path(), &run.run_id).unwrap();
    assert_eq!(status[0].attempt, 1);
    assert_eq!(status[0].phase, Phase::WaitingRecovery);
}

#[tokio::test]
async fn restart_recovers_committed_success_and_explicit_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.begin("process", None).unwrap();
    crate::step_io::write_execution_artifacts(
        tmp.path(),
        &run.run_id,
        "process",
        None,
        1,
        &done(StepOutcome::Done).result,
    )
    .await
    .unwrap();
    let restored = Managed::open(tmp.path(), &run, true).unwrap();
    assert!(!restored.waiting());
    assert_eq!(restored.rows[&("process".into(), None)].phase, Phase::Done);
    managed
        .completed(&mut done(StepOutcome::Error), false)
        .await
        .unwrap();
    control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Fail, "finish-failure", 1),
    )
    .unwrap();
    let mut completions = managed.poll(&BTreeMap::new(), false).unwrap();
    managed
        .completed(&mut completions.remove(0), false)
        .await
        .unwrap();
    let mut states = opencoder_dag::StepStates::new();
    crate::checkpoint::restore(
        tmp.path(),
        &run,
        &mut states,
        &mut opencoder_dag::StepOutputs::new(),
    )
    .unwrap();
    assert_eq!(states["process"], StepOutcome::Error);
}

#[tokio::test]
async fn run_cancel_clears_inactive_holds_and_preserves_completed_states() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.begin("process", None).unwrap();
    managed
        .completed(&mut done(StepOutcome::Error), false)
        .await
        .unwrap();
    managed.cancel_pending(&BTreeMap::new(), &run).unwrap();
    assert!(!managed.waiting());
    assert_eq!(
        control::status(tmp.path(), &run.run_id).unwrap()[0].phase,
        Phase::Cancelled
    );
}

#[tokio::test]
async fn group_pause_waits_for_every_live_child_and_resume_keeps_completed_children() {
    let tmp = tempfile::tempdir().unwrap();
    let mut run = run();
    run.spec.steps[0].kind=serde_json::from_value(json!({"type":"dynamic","source":{"type":"input","pointer":"/items"},"template":{"type":"binary","resource":"fixture"}})).unwrap();
    let mut managed = Managed::open(tmp.path(), &run, false).unwrap();
    managed.instances("process", 2).unwrap();
    managed.begin("process", Some(0)).unwrap();
    managed.begin("process", Some(1)).unwrap();
    let mut successful = done(StepOutcome::Done);
    successful.instance = Some(1);
    managed.completed(&mut successful, false).await.unwrap();
    let token = CancellationToken::new();
    let active = BTreeMap::from([(("process".into(), Some(0)), token.clone())]);
    control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Pause, "pause-group", 1),
    )
    .unwrap();
    managed.poll(&active, false).unwrap();
    assert!(token.is_cancelled());
    let mut stopped = done(StepOutcome::Cancelled);
    stopped.instance = Some(0);
    assert!(managed.completed(&mut stopped, false).await.unwrap());
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "pause-group")
            .unwrap()
            .unwrap()
            .phase,
        "queued"
    );
    managed.poll(&BTreeMap::new(), false).unwrap();
    assert_eq!(
        control::receipt(tmp.path(), &run.run_id, "pause-group")
            .unwrap()
            .unwrap()
            .phase,
        "applied"
    );
    assert!(!managed.group_ready("process"));
    control::submit(
        tmp.path(),
        &run.run_id,
        &run.spec,
        command(Action::Resume, "resume-group", 1),
    )
    .unwrap();
    managed.poll(&BTreeMap::new(), false).unwrap();
    assert!(managed.ready("process", Some(0)));
    assert!(!managed.ready("process", Some(1)));
    let rows = control::status(tmp.path(), &run.run_id).unwrap();
    assert_eq!(
        rows.iter().find(|r| r.instance == Some(1)).unwrap().phase,
        Phase::Done
    );
}
