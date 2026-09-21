//! `GET /api/brain/runs/:id/layered` and `/layered/rounds/:round`: the view
//! keys the workbench reads, and the rule that a layered route never serves
//! another schema version.
use super::*;

#[tokio::test]
async fn layered_view_and_rounds_read_the_node_projection() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    create(&h).await;
    h.node.set_brain("snapshot", 200, snapshot("waiting", 1, 3));
    let path = format!("/api/brain/runs/{RUN}/layered");
    let (status, view) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{view}");
    assert_eq!(view["schema_version"], json!(4));
    assert_eq!(view["run"]["run_id"], json!(RUN));
    assert_eq!(view["run"]["phase"], json!("waiting"));
    assert_eq!(view["run"]["layer"], json!(1));
    assert_eq!(view["run"]["generation"], json!(3));
    assert_eq!(view["run"]["total_layers"], json!(2));
    assert_eq!(view["layers"], json!([["scan"], ["apply"]]));
    assert_eq!(view["operations"], json!([]));
    assert_eq!(view["events"], json!([]));
    assert_eq!(view["plan"]["nodes"].as_array().unwrap().len(), 2);
    let capabilities = view["capabilities"].as_array().unwrap();
    assert_eq!(capabilities.len(), 2);
    assert_eq!(capabilities[0]["capability_id"], json!("builtin-agent-act"));
    assert!(capabilities.iter().all(|capability| {
        capability["kind"].is_string()
            && capability["target"].is_string()
            && capability["version"].is_string()
    }));

    for (layer, node, attempts) in [(1, "scan", 2), (2, "apply", 3)] {
        let path = format!("/api/brain/runs/{RUN}/layered/rounds/{layer}");
        let (status, round) = h.req(Method::GET, &path, None).await;
        assert_eq!(status, 200, "{round}");
        assert_eq!(round["schema_version"], json!(4));
        assert_eq!(round["layer"], json!(layer));
        assert_eq!(round["phase"], json!("waiting"));
        assert_eq!(round["evidence_execution_ids"], json!([]));
        let nodes = round["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["node_id"], json!(node));
        assert_eq!(nodes[0]["status"], json!("pending"));
        assert_eq!(nodes[0]["attempt"], json!(0));
        assert_eq!(nodes[0]["attempts"], json!(attempts));
        assert_eq!(nodes[0]["cancel_requested"], json!(false));
        assert_eq!(nodes[0]["inputs"], json!({}));
        assert!(nodes[0]["execution_id"].is_null());
    }
    // Layers are derived from the plan, so an out-of-range round is a miss.
    for round in [0, 3] {
        let path = format!("/api/brain/runs/{RUN}/layered/rounds/{round}");
        let (status, body) = h.req(Method::GET, &path, None).await;
        assert_eq!(status, 404, "{round}: {body}");
    }
    let reads = h
        .node
        .brain_calls()
        .iter()
        .filter(|call| call.action == "snapshot")
        .count();
    assert!(
        reads >= 3,
        "every layered read is a node projection read: {reads}"
    );
}

#[tokio::test]
async fn layered_events_and_snapshot_routes_delegate_for_v4_runs() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    create(&h).await;
    // The workbench run stream is the shared execution event SSE; the owning
    // node pages the layered run's own log there, so the relay only has to
    // forward the frames and close once the node reports `finished`.
    h.node.set_events(
        RUN,
        vec![json!({"seq":1,"kind":"layered_phase",
            "data":{"run_id":RUN,"layer":1,"phase":"deciding","at_ms":7},"ts":7})],
        true,
    );
    h.node
        .set_brain("snapshot", 200, snapshot("deciding", 1, 1));
    let path = format!("/api/brain/runs/{RUN}/events?after=0");
    let (status, text) = h.sse_text(&path).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{text}");
    assert!(text.contains("event: layered_phase"), "{text}");
    assert!(text.contains("id: 1"), "{text}");
    assert!(text.contains("deciding"), "{text}");
    assert!(text.contains("stream_end"), "{text}");
    // The paged detail route is the projected layered snapshot.
    let path = format!("/api/brain/runs/{RUN}?offset=0");
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["schema_version"], json!(4));
    assert_eq!(body["run"]["run_id"], json!(RUN));
    assert_eq!(body["run"]["phase"], json!("deciding"));
}

#[tokio::test]
async fn layered_routes_never_cross_serve_another_schema() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    create(&h).await;
    seed_v3_run(&h, "brain-scheduler-e2e").await;
    // If a v3 or unknown run were served, this projection would answer it.
    h.node.set_brain("snapshot", 200, snapshot("ready", 0, 1));
    for path in [
        "/api/brain/runs/brain-scheduler-e2e/layered".to_string(),
        "/api/brain/runs/brain-scheduler-e2e/layered/rounds/1".to_string(),
        "/api/brain/runs/brain-missing/layered".to_string(),
        "/api/brain/runs/brain-missing/layered/rounds/1".to_string(),
    ] {
        let (status, body) = h.req(Method::GET, &path, None).await;
        assert_eq!(status, 404, "{path}: {body}");
    }
    // The v3 presentation routes refuse a layered run instead of rendering it.
    for path in [
        format!("/api/brain/runs/{RUN}/view"),
        format!("/api/brain/runs/{RUN}/rounds/1"),
    ] {
        let (status, body) = h.req(Method::GET, &path, None).await;
        assert_eq!(status, 409, "{path}: {body}");
        assert!(body.to_string().contains("migration required"), "{body}");
    }
}
