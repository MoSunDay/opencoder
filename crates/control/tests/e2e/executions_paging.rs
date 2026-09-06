//! Deep-observation paging endpoints: messages, todo items, project runs,
//! team turns (all node-owned JSON) and the chunked binary artifact stream.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

#[tokio::test]
async fn messages_endpoint_pages_node_owned_sessions() {
    let h = Harness::new().await;
    h.put_index("agent-msg-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_messages(
        "agent-msg-1",
        json!({
            "chunks": [{"seq": 1, "role": "user", "offset": 0, "next_offset": 4,
                        "total_bytes": 4, "eof": true, "encoding": "base64", "bytes_b64": "dXNlcg=="}],
            "next_cursor": {"seq": 2, "offset": 0},
            "more": true,
        }),
    );
    let (status, body) = h
        .req(Method::GET, "/api/executions/agent-msg-1/messages", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["chunks"][0]["role"], json!("user"));
    assert_eq!(body["next_cursor"]["seq"], json!(2));
    assert_eq!(body["more"], json!(true));
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-msg-1/messages?seq=1&offset=4",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // Cursor validation: negative seq / offset without seq.
    for query in ["seq=-1", "offset=4"] {
        let (status, body) = h
            .req(
                Method::GET,
                &format!("/api/executions/agent-msg-1/messages?{query}"),
                None,
            )
            .await;
        assert_eq!(status, 400, "{query}: {body}");
    }
    let (status, body) = h
        .req(Method::GET, "/api/executions/agent-none/messages", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn todo_project_and_team_observation_endpoints() {
    let h = Harness::new().await;
    h.put_index(
        "todos-obs-1",
        ExecutionKind::Todos,
        ExecutionStatus::Running,
    )
    .await;
    h.node.set_todo_items(
        "todos-obs-1",
        json!({"items": [{"id": "t1"}], "next_ordinal": 2, "more": false}),
    );
    h.put_index(
        "project-obs-1",
        ExecutionKind::Project,
        ExecutionStatus::Running,
    )
    .await;
    h.node.set_project_runs(
        "project-obs-1",
        json!({"runs": [{"version": 1}], "next_version": null, "more": false}),
    );
    h.put_index("team-obs-1", ExecutionKind::Team, ExecutionStatus::Running)
        .await;
    h.node.set_team_turns(
        "team-obs-1",
        json!({"turns": [{"turn": 1}], "next_turn": 2, "more": true}),
    );

    let (status, body) = h
        .req(Method::GET, "/api/executions/todos-obs-1/todo-items", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["items"][0]["id"], json!("t1"));
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/todos-obs-1/todo-items?after_ordinal=-2",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/project-obs-1/project-runs",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["runs"][0]["version"], json!(1));
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/project-obs-1/project-runs?before_version=0",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/team-obs-1/team-turns?after_turn=1",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["turns"][0]["turn"], json!(1));
    assert_eq!(body["more"], json!(true));
}

#[tokio::test]
async fn artifact_streams_chunked_bytes_for_dag_runs_only() {
    let h = Harness::new().await;
    h.put_index("dag-art-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    // > 64 KiB forces the multi-chunk path; content is positional so any
    // reassembly bug shows up immediately.
    let payload: Vec<u8> = (0..130_000).map(|i| (i % 251) as u8).collect();
    h.node
        .set_artifact("dag-art-1", "build", "report.bin", payload.clone());

    let resp = h
        .req_raw(
            Method::GET,
            "/api/executions/dag-art-1/artifact?step=build&file=report.bin",
            None,
            Some(crate::support::TOKEN),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );
    assert_eq!(
        resp.headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap(),
        "attachment; filename=\"dag-art-1-build-report.bin\""
    );
    let bytes = resp.bytes().await.unwrap();
    assert_eq!(bytes.len(), payload.len(), "chunked reassembly length");
    assert_eq!(&bytes[..], &payload[..], "chunked reassembly content");

    // Missing step → axum query rejection.
    let (status, _) = h
        .req(Method::GET, "/api/executions/dag-art-1/artifact", None)
        .await;
    assert_eq!(status, 400);
    // Non-DAG executions are rejected by the control plane.
    h.put_index("agent-art-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/agent-art-1/artifact?step=build",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("artifacts require a DAG execution"));
    // Unknown artifact on the node → passthrough 404.
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/dag-art-1/artifact?step=missing",
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
}

/// Table of invalid list queries: limit above the 500 cap, and cursors that
/// are half-present or carry a malformed id.
#[tokio::test]
async fn list_rejects_out_of_range_limit_and_half_cursors() {
    let h = Harness::new().await;
    h.put_index("agent-bound-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    for query in [
        "limit=501",
        "cursor_created_at=1",
        "cursor_created_at=1&cursor_id=bad!id",
        "cursor_id=agent-bound-1",
    ] {
        let (status, body) = h
            .req(Method::GET, &format!("/api/executions?{query}"), None)
            .await;
        assert_eq!(status, 400, "{query}: {body}");
    }
}

#[tokio::test]
async fn list_walks_all_pages_through_next_cursor() {
    let h = Harness::new().await;
    for id in ["agent-walk-3", "agent-walk-1", "agent-walk-2"] {
        h.put_index(id, ExecutionKind::Agent, ExecutionStatus::Idle)
            .await;
    }

    let mut pages: Vec<Vec<String>> = Vec::new();
    let mut query = "?limit=1".to_string();
    loop {
        let (status, body) = h
            .req(Method::GET, &format!("/api/executions{query}"), None)
            .await;
        assert_eq!(status, 200, "{body}");
        let rows = body["executions"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{body}");
        pages.push(
            rows.iter()
                .map(|row| row["id"].as_str().unwrap().to_string())
                .collect(),
        );
        match body["next_cursor"].as_object() {
            Some(cursor) => {
                let created_at = cursor["created_at"].as_i64().expect("cursor created_at");
                let id = cursor["id"].as_str().expect("cursor id");
                assert!(pages.len() < 3, "paging must terminate: {body}");
                query = format!("?limit=1&cursor_created_at={created_at}&cursor_id={id}");
            }
            // Final page: the cursor is absent (skipped when None).
            None => break,
        }
    }

    assert_eq!(pages.len(), 3, "{pages:?}");
    let flat: Vec<&String> = pages.iter().flatten().collect();
    assert_eq!(flat.len(), 3, "one row per page: {pages:?}");
    let mut unique = flat.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "pages must be disjoint: {pages:?}");
    for want in ["agent-walk-1", "agent-walk-2", "agent-walk-3"] {
        assert!(unique.iter().any(|id| id.as_str() == want), "{pages:?}");
    }
}
