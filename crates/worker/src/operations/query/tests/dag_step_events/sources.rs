//! Step event source selection: an `agent` step streams its own child
//! session, while `wasm`/`runner` steps are filtered out of the run session.

use super::*;
use serde_json::{json, Value};

#[tokio::test]
async fn dag_step_events_agent_step_streams_its_child_session() {
    let (_root, worker) = worker().await;
    let run = "dag-step-agent";
    let child = "01HZDAGSTEPCHILD0000000001";
    dag_run(
        &worker,
        run,
        vec![step_spec("plan", "agent"), step_spec("build", "wasm")],
    )
    .await;
    session(&worker, run).await;
    session(&worker, child).await;
    artifacts(
        &worker,
        run,
        "plan",
        &[("session.json", json!({"session_id": child}))],
    );
    // Run-level frames for the same step never leak into the child view.
    append(&worker, run, vec![("step_started", json!({"step":"plan"}))]).await;
    let seqs = append(
        &worker,
        child,
        vec![
            // No `step` field: a child session needs no filtering.
            ("status", json!({"text":"thinking"})),
            ("message", json!({"step":"other","text":"kept"})),
        ],
    )
    .await;

    let reply = dag_step_events(&worker, &dag_ref(run), "plan", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(
        frames(&reply)
            .iter()
            .map(|frame| (frame["seq"].clone(), frame["kind"].clone()))
            .collect::<Vec<_>>(),
        vec![
            (json!(seqs[0]), json!("status")),
            (json!(seqs[1]), json!("message"))
        ]
    );
    assert_eq!(frames(&reply)[0]["data"], json!({"text":"thinking"}));
    assert!(frames(&reply)[0]["ts"].is_i64());
    assert_eq!(reply.body["more"], json!(false));
    assert_eq!(reply.body["finished"], json!(false));
    assert_eq!(reply.body["head_seq"], json!(*seqs.last().unwrap()));
    assert_eq!(reply.body["step"]["name"], json!("plan"));
    assert_eq!(reply.body["step"]["kind"], json!("agent"));
    assert_eq!(reply.body["step"]["status"], json!("pending"));
    assert_eq!(reply.body["step"]["session_id"], json!(child));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_wasm_step_keeps_only_its_own_step_output() {
    let (_root, worker) = worker().await;
    let run = "dag-step-wasm";
    dag_run(
        &worker,
        run,
        vec![step_spec("fetch", "wasm"), step_spec("load", "wasm")],
    )
    .await;
    session(&worker, run).await;
    let seqs = append(
        &worker,
        run,
        vec![
            (
                "step_output",
                json!({"step":"fetch","stream":"stdout","text":"a"}),
            ),
            (
                "step_output",
                json!({"step":"load","stream":"stdout","text":"b"}),
            ),
            ("step_started", json!({"step":"fetch"})),
            (
                "step_output",
                json!({"step":"fetch","stream":"stderr","text":"c"}),
            ),
        ],
    )
    .await;

    let reply = dag_step_events(&worker, &dag_ref(run), "fetch", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    // Other steps and non-output kinds are dropped, but the surviving frames
    // keep their run-session seq so the client cursor still advances.
    assert_eq!(
        frames(&reply)
            .iter()
            .map(|frame| frame["seq"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![seqs[0], seqs[3]]
    );
    assert_eq!(frames(&reply)[1]["data"]["stream"], json!("stderr"));
    assert_eq!(reply.body["head_seq"], json!(*seqs.last().unwrap()));
    assert_eq!(reply.body["step"]["kind"], json!("wasm"));
    assert!(reply.body["step"].get("session_id").is_none(), "{reply:?}");

    // The cursor is honored: everything at or below seq 1 is behind us.
    let tail = dag_step_events(&worker, &dag_ref(run), "fetch", seqs[0])
        .await
        .unwrap();
    assert_eq!(
        frames(&tail)
            .iter()
            .map(|frame| frame["seq"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![seqs[3]]
    );
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_runner_step_filters_by_step_only() {
    let (_root, worker) = worker().await;
    let run = "dag-step-runner";
    dag_run(
        &worker,
        run,
        vec![step_spec("review", "runner"), step_spec("other", "runner")],
    )
    .await;
    session(&worker, run).await;
    append(
        &worker,
        run,
        vec![
            ("runner_stage", json!({"step":"review","stage":"decode"})),
            ("codex_token", json!({"step":"review","text":"x"})),
            ("runner_stage", json!({"step":"other","stage":"decode"})),
            ("runner_stage", json!({"stage":"no step field"})),
        ],
    )
    .await;

    let reply = dag_step_events(&worker, &dag_ref(run), "review", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(
        frames(&reply)
            .iter()
            .map(|frame| frame["kind"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        vec!["runner_stage".to_string(), "codex_token".to_string()]
    );
    assert_eq!(reply.body["step"]["kind"], json!("runner"));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_step_events_keeps_polling_before_the_step_session_exists() {
    let (_root, worker) = worker().await;
    let run = "dag-step-pending";
    dag_run(&worker, run, vec![step_spec("plan", "agent")]).await;
    session(&worker, run).await;
    append(
        &worker,
        run,
        vec![
            ("step_started", json!({"step":"plan"})),
            ("step_started", json!({"step":"other"})),
        ],
    )
    .await;

    // No `session.json`/`meta.json` yet: the run session is the only source
    // and the stream stays open instead of terminating or 404-ing.
    let reply = dag_step_events(&worker, &dag_ref(run), "plan", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(frames(&reply).len(), 1, "{reply:?}");
    assert_eq!(frames(&reply)[0]["kind"], json!("step_started"));
    assert_eq!(reply.body["finished"], json!(false));
    assert_eq!(reply.body["step"]["status"], json!("pending"));
    assert_eq!(reply.body["step"]["error"], Value::Null);
    assert!(reply.body["step"].get("session_id").is_none(), "{reply:?}");

    // A run whose session has not been created at all is still answerable.
    let cold = "dag-step-cold";
    dag_run(&worker, cold, vec![step_spec("plan", "agent")]).await;
    let reply = dag_step_events(&worker, &dag_ref(cold), "plan", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert!(frames(&reply).is_empty(), "{reply:?}");
    assert_eq!(reply.body["more"], json!(false));
    assert_eq!(reply.body["finished"], json!(false));
    assert_eq!(reply.body["head_seq"], json!(0));

    // The committed receipt can name the child session before its first event;
    // a terminal step then closes the stream with the terminal frame alone.
    let child = "01HZDAGSTEPCHILD0000000003";
    let late = "dag-step-late";
    dag_run(&worker, late, vec![step_spec("plan", "agent")]).await;
    artifacts(
        &worker,
        late,
        "plan",
        &[(
            "meta.json",
            json!({"outcome": "done", "session_id": child, "finished_at_ms": 4}),
        )],
    );
    let reply = dag_step_events(&worker, &dag_ref(late), "plan", 0)
        .await
        .unwrap();
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(frames(&reply).len(), 1, "{reply:?}");
    assert_eq!(frames(&reply)[0]["kind"], json!("step_finished"));
    assert_eq!(frames(&reply)[0]["seq"], json!(1));
    assert_eq!(frames(&reply)[0]["data"]["status"], json!("done"));
    assert_eq!(reply.body["finished"], json!(true));
    assert_eq!(reply.body["step"]["session_id"], json!(child));
    worker.shutdown().await.unwrap();
}
