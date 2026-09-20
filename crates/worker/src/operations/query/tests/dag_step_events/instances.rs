use super::*;
use crate::operations::query::instances;

async fn dynamic_run(worker: &Worker, run: &str, kind: &str, count: usize) {
    let template = if kind == "agent" {
        json!({"type":"agent","prompt":"review"})
    } else {
        json!({"type":"wasm","command":"x.wasm"})
    };
    dag_run(
        worker,
        run,
        vec![json!({"name":"process","kind":{"type":"dynamic",
        "source":{"type":"input","pointer":"/items"},"template":template}})],
    )
    .await;
    let input = if kind == "agent" {
        json!("instruction")
    } else {
        json!(["--title", "hello world"])
    };
    artifacts(
        worker,
        run,
        "process",
        &[
            ("instances.json", json!(vec![input; count])),
            (
                "progress.json",
                json!({"instances":{"total":count,"done":0,"running":0,"pending":count,"error":0,"cancelled":0},"at_ms":1}),
            ),
        ],
    );
}
fn instance_artifacts(worker: &Worker, run: &str, index: usize, files: &[(&str, Value)]) {
    let root = worker.inner.layout.kind_root(ExecutionKind::Dag);
    let dir = opencoder_dag::artifacts::execution_dir(&root, run, "process", Some(index)).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    for (name, value) in files {
        std::fs::write(dir.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn thousand_instances_page_cap_detail_and_identity_validation() {
    let (_tmp, worker) = worker().await;
    dynamic_run(&worker, "dag-instances", "wasm", 1000).await;
    let r = dag_ref("dag-instances");
    let page = instances::query(&worker, &r, "process", None, 0, 100)
        .await
        .unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(page.body["total"], 1000);
    assert_eq!(page.body["instances"].as_array().unwrap().len(), 100);
    assert_eq!(page.body["instances"][99]["index"], 99);
    let page = instances::query(&worker, &r, "process", None, 850, 9999)
        .await
        .unwrap();
    assert_eq!(page.body["limit"], 200);
    assert_eq!(page.body["instances"].as_array().unwrap().len(), 150);
    assert_eq!(page.body["more"], false);
    let detail = instances::query(&worker, &r, "process", Some(999), 0, 100)
        .await
        .unwrap();
    assert_eq!(detail.body["input"], json!(["--title", "hello world"]));
    assert_eq!(detail.body["kind"], "wasm");
    assert_eq!(
        instances::query(&worker, &r, "process", Some(1000), 0, 100)
            .await
            .unwrap()
            .status,
        404
    );
    assert_eq!(
        instances::query(&worker, &r, "../escape", None, 0, 100)
            .await
            .unwrap()
            .status,
        404
    );
}

#[tokio::test]
async fn instance_logs_filter_index_replay_cursor_and_agent_session() {
    let (_tmp, worker) = worker().await;
    let run = "dag-instance-logs";
    dynamic_run(&worker, run, "wasm", 2).await;
    session(&worker, run).await;
    let seqs = append(
        &worker,
        run,
        vec![
            (
                "step_output",
                json!({"step":"process","index":0,"text":"zero"}),
            ),
            (
                "step_output",
                json!({"step":"process","index":1,"text":"one"}),
            ),
            (
                "step_log",
                json!({"step":"process","payload":{"index":1,"event":"stdout","data":"nested"}}),
            ),
        ],
    )
    .await;
    instance_artifacts(
        &worker,
        run,
        1,
        &[
            ("meta.json", json!({"outcome":"done","finished_at_ms":10})),
            ("output.json", json!({"ok":true})),
        ],
    );
    let page = crate::operations::query::dag_step_events::events(
        &worker,
        &dag_ref(run),
        "process",
        Some(1),
        0,
    )
    .await
    .unwrap();
    assert_eq!(page.status, 200);
    assert!(frames(&page).iter().any(|f| f["data"]["text"] == "one"));
    assert!(!frames(&page).iter().any(|f| f["data"]["text"] == "zero"));
    assert!(frames(&page)
        .iter()
        .any(|f| f["data"]["payload"]["data"] == "nested"));
    let replay = crate::operations::query::dag_step_events::events(
        &worker,
        &dag_ref(run),
        "process",
        Some(1),
        seqs[1],
    )
    .await
    .unwrap();
    assert!(!frames(&replay).iter().any(|f| f["data"]["text"] == "one"));
    assert_eq!(page.body["finished"], true);
    instance_artifacts(
        &worker,
        run,
        1,
        &[("meta.json", json!({"outcome":"running","started_at_ms":3}))],
    );
    let retry = crate::operations::query::dag_step_events::events(
        &worker,
        &dag_ref(run),
        "process",
        Some(1),
        0,
    )
    .await
    .unwrap();
    assert_eq!(
        frames(&retry).len(),
        1,
        "retry excludes output from the old attempt"
    );
    assert_eq!(frames(&retry)[0]["data"]["payload"]["data"], "nested");
    instance_artifacts(
        &worker,
        run,
        1,
        &[("meta.json", json!({"outcome":"pending"}))],
    );
    let pending = crate::operations::query::dag_step_events::events(
        &worker,
        &dag_ref(run),
        "process",
        Some(1),
        0,
    )
    .await
    .unwrap();
    assert!(frames(&pending).is_empty());
    assert_eq!(pending.body["head_seq"], 0);
    let run = "dag-instance-agent";
    dynamic_run(&worker, run, "agent", 2).await;
    session(&worker, "instance-one").await;
    session(&worker, "instance-zero").await;
    append(
        &worker,
        "instance-zero",
        vec![("text_delta", json!({"text":"wrong"}))],
    )
    .await;
    append(
        &worker,
        "instance-one",
        vec![("text_delta", json!({"text":"right"}))],
    )
    .await;
    instance_artifacts(
        &worker,
        run,
        1,
        &[
            ("session.json", json!({"session_id":"instance-one"})),
            ("meta.json", json!({"outcome":"running","started_at_ms":20})),
        ],
    );
    let page = crate::operations::query::dag_step_events::events(
        &worker,
        &dag_ref(run),
        "process",
        Some(1),
        0,
    )
    .await
    .unwrap();
    assert_eq!(frames(&page).len(), 1);
    assert_eq!(frames(&page)[0]["data"]["text"], "right");
    assert_eq!(page.body["finished"], false);
}
