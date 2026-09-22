use super::support::*;
use opencoder_dag::DagRunStatus;
use opencoder_dag_runtime::execute_run;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn collect_all_drains_failed_batch_and_runs_finalizer() {
    let f = Fixture::new().await;
    let mut process = dynamic_agent();
    process["kind"]["failure_policy"] = json!("collect_all");
    let mut run = f.run(json!([process,
        {"name":"summary","depends_on":["process"],"trigger_rule":"all_done",
         "kind":{"type":"agent","prompt":"FINALIZE"}}
    ]), json!({"items":["FAIL-FIRST","SUCCEED-SECOND","SUCCEED-THIRD"]}));
    run.spec.max_concurrency = 1;
    let client = Scripted::new(|text| (Duration::ZERO,
        if text.contains("FAIL-FIRST") { Err("original failure".into()) } else { Ok("{}".into()) }));
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(execute_run(f.deps(client.clone()), run.clone(), rx).await.unwrap(), DagRunStatus::Error);
    assert_eq!(client.requests.lock().unwrap().len(), 4);
    assert_eq!(f.json(&run, "process/progress.json")["instances"]["done"], 2);
    assert_eq!(f.json(&run, "summary/meta.json")["outcome"], "done");
    assert_eq!(f.json(&run, "process/instances/0/meta.json")["outcome"], "error");
    let client = Scripted::new(|_| panic!("terminal collect_all instances must not run twice"));
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(opencoder_dag_runtime::resume_run(f.deps(client), run, rx).await.unwrap(), DagRunStatus::Error);
}

#[tokio::test]
async fn prepare_failure_propagates_through_blocked_case_to_finalizer() {
    let f = Fixture::new().await;
    let run = f.run(json!([
        {"name":"summary","depends_on":["cases"],"trigger_rule":"all_done","kind":{"type":"agent","prompt":"FINALIZE"}},
        {"name":"cases","depends_on":["intermediate"],"kind":{"type":"agent","prompt":"NEVER-RUN"}},
        {"name":"intermediate","depends_on":["prepare"],"kind":{"type":"agent","prompt":"NEVER-RUN"}},
        {"name":"prepare","kind":{"type":"agent","prompt":"FAIL-PREPARE"}}
    ]), json!({}));
    let client = Scripted::new(|text| {
        assert!(!text.contains("NEVER-RUN"));
        (Duration::ZERO, if text.contains("FAIL-PREPARE") {Err("preflight failed".into())} else {Ok("{}".into())})
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(execute_run(f.deps(client.clone()), run.clone(), rx).await.unwrap(), DagRunStatus::Error);
    assert_eq!(client.requests.lock().unwrap().len(), 2);
    assert_eq!(f.json(&run, "cases/meta.json")["outcome"], "error");
    assert_eq!(f.json(&run, "summary/meta.json")["outcome"], "done");
}
