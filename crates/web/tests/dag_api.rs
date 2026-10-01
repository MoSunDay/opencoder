//! HTTP contract for the DAG control plane (`/api/dag/*` + `/api/nodes/dag/*`):
//! defs CRUD, dispatch/claim with spec snapshot, event upload validation,
//! SSE replay/resume, terminal status with synthetic `run_finished`, cancel
//! piggyback on the heartbeat. Store-backed (the libsql DAG impl is live);
//! harness mirrors `nodes_ops.rs`.

#[path = "dag_api/support.rs"]
mod support;

use axum::http::StatusCode;
use opencoder_store::DagDefRecord;
use support::{
    app, claim, dispatch, register, req, send, spec_body, spec_body_of, upload, upsert_def,
};
use tower::ServiceExt;

// ── defs CRUD ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn defs_crud_upsert_keeps_id_and_delete_404s() {
    let ctx = app().await;
    let id = upsert_def(&ctx.app).await;

    // Re-publish under the same name: the FIRST row's id survives.
    let (_, again) = send(&ctx.app, req("POST", "/api/dag/defs", Some(spec_body()))).await;
    assert_eq!(again["id"].as_str().unwrap(), id);
    assert!(again["updated_at"].as_i64().unwrap() >= again["created_at"].as_i64().unwrap());

    let (s, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "upsert by name, not append"
    );
    assert_eq!(list[0]["spec"]["steps"].as_array().unwrap().len(), 2);

    let (s, one) = send(&ctx.app, req("GET", &format!("/api/dag/defs/{id}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(one["name"], "etl-demo");

    let (s, _) = send(
        &ctx.app,
        req("DELETE", &format!("/api/dag/defs/{id}"), None),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&ctx.app, req("GET", &format!("/api/dag/defs/{id}"), None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(
        &ctx.app,
        req("DELETE", &format!("/api/dag/defs/{id}"), None),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unsupported_defs_fail_closed_on_create_list_and_get() {
    let ctx = app().await;

    // POST a python spec: dedicated 400 message, nothing stored.
    let python_spec =
        r#"{"name":"p","steps":[{"name":"a","kind":{"type":"python","code":"pass"}}]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(python_spec))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("unknown variant `python`"),
        "{b}"
    );
    let (_, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    assert_eq!(list.as_array().unwrap().len(), 0);

    // Seed a legacy python row straight through the store (the HTTP path
    // can no longer produce one): the list degrades to an error row.
    ctx.store
        .upsert_dag_def(&DagDefRecord {
            id: "legacy-python".into(),
            name: "legacy-python".into(),
            spec_json: python_spec.to_string(),
            created_at: 1,
            updated_at: 1,
        })
        .await
        .unwrap();
    let (s, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["spec"].is_null(), "{list}");
    let err = rows[0]["error"].as_str().unwrap();
    assert!(err.contains("unknown variant `python`"), "{err}");
    // The degraded row keeps its identity: flatten-None used to drop
    // id/name entirely, sending the SPA delete button to `…/defs/undefined`.
    assert_eq!(rows[0]["id"], "legacy-python", "{list}");
    assert_eq!(rows[0]["name"], "legacy-python", "{list}");
    assert_eq!(rows[0]["created_at"], 1, "{list}");
    assert_eq!(rows[0]["updated_at"], 1, "{list}");

    // get_def stays fail-closed with the same dedicated message.
    let (s, one) = send(&ctx.app, req("GET", "/api/dag/defs/legacy-python", None)).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{one}");
    assert!(
        one["error"]
            .as_str()
            .unwrap()
            .contains("unknown variant `python`"),
        "{one}"
    );

    // A decodable def lists without the error field and keeps its spec.
    let _ = upsert_def(&ctx.app).await;
    let (_, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    let good = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "etl-demo")
        .unwrap();
    assert!(good.get("error").is_none(), "{list}");
    assert_eq!(good["spec"]["steps"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn invalid_spec_is_400_with_the_problem_list() {
    let ctx = app().await;
    let bad = r#"{"name":"x","steps":[]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(bad))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("spec.steps must not be empty"),
        "{b}"
    );

    let bad_slug =
        r#"{"name":"x","steps":[{"name":"Bad Slug","kind":{"type":"binary","resource":"tool"}}]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(bad_slug))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("not a valid slug"),
        "{b}"
    );

    let (_, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        0,
        "rejected specs are not stored"
    );
}

/// `max_concurrency` round-trips through def storage (always serialized, so
/// the SPA mirror stays honest); out-of-range values are a 400 with the
/// aggregated problem list naming the field.
#[tokio::test]
async fn def_upsert_roundtrips_max_concurrency_and_rejects_out_of_range() {
    let ctx = app().await;
    let ok = r#"{"name":"conc","max_concurrency":8,"steps":[
        {"name":"fetch","kind":{"type":"binary","resource":"tool"}}]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(ok))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let id = b["id"].as_str().unwrap().to_string();

    let (s, one) = send(&ctx.app, req("GET", &format!("/api/dag/defs/{id}"), None)).await;
    assert_eq!(s, StatusCode::OK, "{one}");
    assert_eq!(one["spec"]["max_concurrency"], 8, "{one}");

    let (_, list) = send(&ctx.app, req("GET", "/api/dag/defs", None)).await;
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "conc")
        .unwrap();
    assert_eq!(row["spec"]["max_concurrency"], 8, "{list}");

    let bad = r#"{"name":"conc","max_concurrency":31,"steps":[
        {"name":"fetch","kind":{"type":"binary","resource":"tool"}}]}"#;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs", Some(spec_body_of(bad))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("max_concurrency must be 1..=30"),
        "{b}"
    );
}

// ── dispatch + claim ───────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_unknown_def_404_and_unknown_node_400() {
    let ctx = app().await;
    let (s, b) = send(
        &ctx.app,
        req("POST", "/api/dag/defs/01GHOST/dispatch", Some("{}".into())),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");

    let def_id = upsert_def(&ctx.app).await;
    let (s, b) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/dag/defs/{def_id}/dispatch"),
            Some(r#"{"node_id":"01NOPE"}"#.into()),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("does not exist"),
        "{b}"
    );
}

#[tokio::test]
async fn claim_returns_spec_snapshot_and_second_claim_is_204() {
    let ctx = app().await;
    let node = register(&ctx.app, "worker-1").await;
    let def_id = upsert_def(&ctx.app).await;
    let rid = dispatch(&ctx.app, &def_id, Some(&node)).await;

    let claimed = claim(&ctx.app, &node).await.expect("run was due");
    assert_eq!(claimed["run_id"].as_str().unwrap(), rid);
    assert_eq!(claimed["dag_id"].as_str().unwrap(), def_id);
    let steps = claimed["spec"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "claim carries the spec snapshot");
    assert_eq!(steps[1]["depends_on"][0], "fetch");

    // Single-active-run policy: the busy node gets nothing more.
    assert!(claim(&ctx.app, &node).await.is_none());

    let (s, run) = send(&ctx.app, req("GET", &format!("/api/dag/runs/{rid}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(run["status"], "running");
    assert_eq!(run["node_id"], node.as_str());
    assert_eq!(run["name"], "etl-demo");
    assert!(run["claimed_at"].is_i64());

    // Newest-first listing sees the run, with the spec name at top level.
    let (_, runs) = send(&ctx.app, req("GET", "/api/dag/runs?limit=5", None)).await;
    let rows = runs.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "etl-demo", "{runs}");
}

// ── event upload validation ────────────────────────────────────────────────

#[tokio::test]
async fn event_upload_validates_run_kind_and_run_id() {
    let ctx = app().await;
    let node = register(&ctx.app, "worker-2").await;
    let def_id = upsert_def(&ctx.app).await;
    let rid = dispatch(&ctx.app, &def_id, Some(&node)).await;
    let _ = claim(&ctx.app, &node).await;

    let (s, b) = upload(&ctx.app, "01GHOST", serde_json::json!([])).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");

    let mismatch = serde_json::json!([{"kind":"run_started","at_ms":1}]);
    let resp = ctx
        .app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/nodes/dag/runs/{rid}/events"),
            Some(serde_json::json!({"run_id":"other","events":mismatch}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([{"kind":"step_exploded","step":"fetch","at_ms":1}]),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("unknown dag event kind"),
        "{b}"
    );

    // Rejected batches leave nothing behind.
    let persisted = ctx.store.dag_events_after(&rid, 0, 100).await.unwrap();
    assert!(persisted.is_empty());

    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([
            {"kind":"run_started","at_ms":1},
            {"kind":"step_started","step":"fetch","at_ms":2}
        ]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["accepted"], 2);
}

/// A terminal run closes its event stream: uploads after the terminal status
/// report are 409'd and nothing new is persisted (the only frame the
/// terminal move adds is the store's own synthetic `run_finished`).
#[tokio::test]
async fn events_rejected_after_terminal_status() {
    let ctx = app().await;
    let node = register(&ctx.app, "worker-3").await;
    let def_id = upsert_def(&ctx.app).await;
    let rid = dispatch(&ctx.app, &def_id, Some(&node)).await;
    let _ = claim(&ctx.app, &node).await;

    // Live run: uploads land.
    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([{"kind":"run_started","at_ms":1}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let before = ctx
        .store
        .dag_events_after(&rid, 0, 100)
        .await
        .unwrap()
        .len();

    // Terminal report, same envelope the node sends.
    let (s, b) = send(
        &ctx.app,
        req(
            "POST",
            &format!("/api/nodes/dag/runs/{rid}/status"),
            Some(format!(r#"{{"run_id":"{rid}","status":"done"}}"#)),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Post-terminal upload: 409, and the stream must not grow.
    let (s, b) = upload(
        &ctx.app,
        &rid,
        serde_json::json!([{"kind":"step_done","step":"fetch","at_ms":2}]),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(
        b["error"].as_str().unwrap().contains("event stream closed"),
        "{b}"
    );
    let after = ctx.store.dag_events_after(&rid, 0, 100).await.unwrap();
    assert_eq!(
        after.len(),
        before + 1,
        "only the synthetic run_finished frame was appended"
    );
    assert_eq!(after.last().unwrap().kind, "run_finished");
    assert_eq!(after.last().unwrap().payload["status"], "done");
}
