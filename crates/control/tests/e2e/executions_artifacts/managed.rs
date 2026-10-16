use crate::support::{Harness, TOKEN};
use opencoder_core::fleet::*;
use reqwest::Method;
use serde_json::{json, Value};

#[tokio::test]
async fn managed_agent_team_and_todo_artifacts_stream_while_ordinary_executions_stay_rejected() {
    let h = Harness::new().await;
    for kind in [
        ExecutionKind::Agent,
        ExecutionKind::Team,
        ExecutionKind::Todos,
    ] {
        let id = format!("{}-managed-artifact", kind.prefix());
        let assignment = Assignment {
            private_context: None,
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: id.clone(),
                kind,
                node_id: h.node.id.clone(),
                created_at: 1,
                status: ExecutionStatus::Done,
            },
            request: CreateExecution {
                id: id.clone(),
                kind,
                target: None,
                node_id: None,
                input: json!({"schema_version":7,"brain_layered":{"run_id":"brain-files"}}),
            },
            definition: None,
        };
        let fingerprint = opencoder_core::token_hash(&assignment.request.input.to_string());
        assert!(h
            .state
            .fleet
            .claim_request("execution", &id, &fingerprint)
            .await
            .unwrap());
        h.state
            .fleet
            .prepare_assignment(&assignment, &fingerprint)
            .await
            .unwrap();
        h.state
            .fleet
            .finish_dispatch(&id, &fingerprint, &RpcReply::ok(json!(assignment.index)))
            .await
            .unwrap();
        h.node
            .set_artifact(&id, "workspace", "report.txt", b"verified file".to_vec());
        let response = h
            .req_raw(
                Method::GET,
                &format!("/api/executions/{id}/artifact?step=workspace&file=report.txt"),
                None,
                Some(TOKEN),
            )
            .await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"verified file");

        let ordinary = format!("{}-ordinary-artifact", kind.prefix());
        h.put_index(&ordinary, kind, ExecutionStatus::Done).await;
        let (status, body) = h
            .req(
                Method::GET,
                &format!("/api/executions/{ordinary}/artifact?step=workspace&file=report.txt"),
                None,
            )
            .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            body["error"],
            Value::String("execution does not expose artifacts".into())
        );
    }
}
