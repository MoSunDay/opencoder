mod finalizers;
mod persistence;
mod recovery;
mod support;
use opencoder_dag::DagRunStatus;
use opencoder_dag_runtime::execute_run;
use serde_json::json;
use std::time::Duration;
use support::*;

#[tokio::test]
async fn agent_inputs_enter_distinct_how_copies_and_outputs_keep_input_order() {
    let f = Fixture::new().await;
    let run = f.run(json!([dynamic_agent(), {"name":"finish","depends_on":["process"],"kind":{"type":"agent","prompt":"downstream"}}]),
        json!({"items":["ITEM-ZERO", "ITEM-ONE"]}));
    let client = Scripted::new(|text| {
        if text.contains("downstream") {
            assert!(text.contains("zero") && text.contains("one"));
        }
        let zero = text.contains("ITEM-ZERO");
        (
            Duration::from_millis(if zero { 100 } else { 1 }),
            Ok(json!({"item":if zero {"zero"} else {"one"}}).to_string()),
        )
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(
        execute_run(f.deps(client.clone()), run.clone(), rx)
            .await
            .unwrap(),
        DagRunStatus::Done
    );
    assert_eq!(
        f.text(&run, "process/instances/0/how.md"),
        "common-how\n\nITEM-ZERO"
    );
    assert_eq!(
        f.text(&run, "process/instances/1/how.md"),
        "common-how\n\nITEM-ONE"
    );
    assert_eq!(
        f.json(&run, "process/output.json"),
        json!([{"item":"zero"},{"item":"one"}])
    );
    let requests = client.requests.lock().unwrap();
    assert!(requests.iter().any(|r| r.contains("ITEM-ZERO")
        && r.contains("common-how")
        && r.contains("common-prompt")
        && !r.contains("ITEM-ONE")));
    assert!(requests
        .iter()
        .any(|r| r.contains("ITEM-ONE") && !r.contains("ITEM-ZERO")));
    assert_ne!(
        f.json(&run, "process/instances/0/session.json")["session_id"],
        f.json(&run, "process/instances/1/session.json")["session_id"]
    );
}

#[tokio::test]
async fn upstream_output_expands_wasm_argv_without_splitting_spaces() {
    let f = Fixture::new().await;
    let run = f.run(json!([
        {"name":"discover","kind":{"type":"agent","prompt":"discover"}},
        {"name":"process","depends_on":["discover"],"kind":{"type":"dynamic","source":{"type":"step_output","step":"discover","pointer":"/items"},"template":{"type":"wasm","command":"args.wasm --format json"}}}
    ]), json!({}));
    f.module(&run, "args.wasm", ARGV_WAT);
    let client = Scripted::new(|_| {
        (
            Duration::ZERO,
            Ok(json!({"items":[["--title","hello world"],[]]}).to_string()),
        )
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(
        execute_run(f.deps(client), run.clone(), rx).await.unwrap(),
        DagRunStatus::Done
    );
    assert_eq!(
        f.text(&run, "process/instances/0/output.txt"),
        "args.wasm\0--format\0json\0--title\0hello world\0"
    );
    assert_eq!(
        f.text(&run, "process/instances/1/output.txt"),
        "args.wasm\0--format\0json\0"
    );
    let rows = f.store.events_after(&run.run_id, 0).await.unwrap();
    assert!(rows.iter().any(|r| r.payload["index"] == 0
        && r.payload["text"]
            .as_str()
            .is_some_and(|s| s.contains("hello world"))));
}

#[tokio::test]
async fn four_slots_round_robin_and_failed_group_do_not_cancel_independent_branch() {
    let f = Fixture::new().await;
    let run = f.run(json!([dynamic_agent(),
        {"name":"independent","kind":{"type":"agent","prompt":"independent-branch"}},
        {"name":"blocked","depends_on":["process"],"kind":{"type":"agent","prompt":"must-not-run"}}
    ]), json!({"items":["FAIL-INSTANCE","slow-1","slow-2","not-dispatched-3","not-dispatched-4"]}));
    let client = Scripted::new(|text| {
        if text.contains("FAIL-INSTANCE") {
            (Duration::from_millis(50), Err("instance failed".into()))
        } else if text.contains("independent-branch") {
            (Duration::from_millis(200), Ok("{}".into()))
        } else {
            (Duration::from_secs(10), Ok("{}".into()))
        }
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(
        execute_run(f.deps(client.clone()), run.clone(), rx)
            .await
            .unwrap(),
        DagRunStatus::Error
    );
    assert_eq!(f.json(&run, "independent/meta.json")["outcome"], "done");
    assert_eq!(f.json(&run, "blocked/meta.json")["outcome"], "error");
    assert_eq!(client.requests.lock().unwrap().len(), 4);
    assert!(!client
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.contains("not-dispatched") || r.contains("must-not-run")));
    let events = f.events.0.lock().unwrap();
    let mut live = 0;
    let mut max = 0;
    for e in events.iter() {
        if e.kind == "instance_started"
            || (e.kind == "step_started" && e.step.as_deref() == Some("independent"))
        {
            live += 1;
            max = max.max(live);
        }
        if e.kind == "instance_done"
            || (e.kind == "step_done" && e.step.as_deref() == Some("independent"))
        {
            live -= 1;
        }
    }
    assert_eq!(max, 4);
    assert_eq!(live, 0);
    assert_eq!(
        f.json(&run, "process/progress.json")["instances"]["cancelled"],
        4
    );
}

#[tokio::test]
async fn empty_batch_succeeds_and_invalid_input_never_partially_expands() {
    let f = Fixture::new().await;
    for (items, expected) in [
        (json!([]), DagRunStatus::Done),
        (json!(["valid", 2]), DagRunStatus::Error),
        (json!(null), DagRunStatus::Error),
        (json!(vec!["x"; 1001]), DagRunStatus::Error),
    ] {
        let run = f.run(json!([dynamic_agent()]), json!({"items":items}));
        let client = Scripted::new(|_| panic!("empty or invalid batch must not execute"));
        let (_, rx) = tokio::sync::watch::channel(false);
        assert_eq!(
            execute_run(f.deps(client), run.clone(), rx).await.unwrap(),
            expected
        );
        if expected == DagRunStatus::Done {
            assert_eq!(f.json(&run, "process/output.json"), json!([]));
        }
        assert!(!f
            .root
            .join(&run.run_id)
            .join("process/instances/0")
            .exists());
    }
}
