//! Playbook dispatch: topological batches, request-scoped idempotency,
//! brain-step target binding resolution, 404/400 edges.

use opencoder_core::fleet::valid_id;
use opencoder_core::fleet::ExecutionKind;
use reqwest::Method;
use serde_json::json;

use super::{agent, index_kind, seed_dag, seed_playbook, spec, step};
use crate::support::Harness;
use opencoder_brain::PlaybookTarget;
use opencoder_brain::PlaybookTrigger;

/// A three-layer graph (a → b1/b2 → c) dispatches one execution per step,
/// batched topologically, and a replay with the same request_id resubmits
/// nothing while returning the identical response.
#[tokio::test]
async fn dispatch_submits_steps_in_topo_batches() {
    let h = Harness::new().await;
    seed_dag(&h).await;
    let spec = spec(
        "playbook-topo",
        PlaybookTrigger::Manual {},
        vec![
            agent("a"),
            step(
                "b1",
                &["a"],
                PlaybookTarget::Agent {
                    agent: "plan".into(),
                },
            ),
            step(
                "b2",
                &["a"],
                PlaybookTarget::Dag {
                    dag: "pbk-dag".into(),
                },
            ),
            step(
                "c",
                &["b1", "b2"],
                PlaybookTarget::Agent {
                    agent: "act".into(),
                },
            ),
        ],
    );
    seed_playbook(&h, &spec).await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-topo/dispatch",
            Some(json!({"request_id": "pbkreq1", "situation": "nightly maintenance"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["playbook_id"], json!("playbook-topo"));
    assert_eq!(
        body["batches"],
        json!([["a"], ["b1", "b2"], ["c"]]),
        "{body}"
    );
    let executions = body["executions"].as_array().cloned().unwrap();
    assert_eq!(executions.len(), 4, "{body}");

    let mut created = Vec::new();
    for entry in &executions {
        let id = entry["id"].as_str().unwrap();
        let index = h.state.fleet.index(id).await.unwrap().expect("indexed");
        assert_eq!(
            serde_json::to_value(index.kind).unwrap(),
            entry["kind"],
            "{entry}"
        );
        assert!(id.starts_with(&format!("{}-pbk-", entry["kind"].as_str().unwrap())));
        assert!(valid_id(id));
        created.push(index.created_at);
    }
    assert_eq!(
        index_kind(&h, "dag-pbk-pbkreq1-b2").await,
        Some(ExecutionKind::Dag)
    );

    // Idempotent replay: same request_id → same ids, nothing resubmitted
    // (created_at of every index survives untouched).
    let (status, replay) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-topo/dispatch",
            Some(json!({"request_id": "pbkreq1", "situation": "nightly maintenance"})),
        )
        .await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay["batches"], body["batches"]);
    assert_eq!(replay["executions"], body["executions"]);
    for (entry, created_at) in executions.iter().zip(created) {
        let id = entry["id"].as_str().unwrap();
        assert_eq!(
            h.state.fleet.index(id).await.unwrap().unwrap().created_at,
            created_at,
            "replay must not resubmit {id}"
        );
    }
}

/// Brain steps resolve the control-plane capability target binding; an
/// unbound capability falls back to the default agent "act".
#[tokio::test]
async fn dispatch_resolves_brain_target_binding() {
    let h = Harness::new().await;
    seed_dag(&h).await;
    let payload = json!({
        "capability_type": "tool-usage",
        "summary": "resize the fleet",
        "input_desc": "a work request",
        "output_desc": "completed work",
        "eng_inputs": ["exemplar input"],
    });
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/capabilities",
            Some(payload.clone()),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let bound = body["capability"]["id"].as_str().unwrap().to_string();
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(payload))
        .await;
    assert_eq!(status, 201, "{body}");
    let unbound = body["capability"]["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{bound}/target"),
            Some(json!({"kind": "dag", "target": "pbk-dag"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    seed_playbook(
        &h,
        &spec(
            "playbook-bound",
            PlaybookTrigger::Manual {},
            vec![
                agent("warm"),
                step(
                    "run",
                    &["warm"],
                    PlaybookTarget::Brain {
                        capability_id: bound.clone(),
                    },
                ),
            ],
        ),
    )
    .await;
    seed_playbook(
        &h,
        &spec(
            "playbook-unbound",
            PlaybookTrigger::Manual {},
            vec![step(
                "run",
                &[],
                PlaybookTarget::Brain {
                    capability_id: unbound.clone(),
                },
            )],
        ),
    )
    .await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-bound/dispatch",
            Some(json!({"request_id": "bindreq"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let run = &body["executions"].as_array().unwrap()[1];
    assert_eq!(run["step"], json!("run"));
    assert_eq!(run["kind"], json!("dag"));
    assert!(run["id"]
        .as_str()
        .unwrap()
        .starts_with("dag-pbk-bindreq-run"));
    assert_eq!(
        index_kind(&h, run["id"].as_str().unwrap()).await,
        Some(ExecutionKind::Dag)
    );

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-unbound/dispatch",
            Some(json!({"request_id": "unbreq"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let run = &body["executions"][0];
    assert_eq!(run["kind"], json!("agent"), "{body}");
    assert!(run["id"]
        .as_str()
        .unwrap()
        .starts_with("agent-pbk-unbreq-run"));
    assert_eq!(
        index_kind(&h, run["id"].as_str().unwrap()).await,
        Some(ExecutionKind::Agent)
    );
}

#[tokio::test]
async fn dispatch_unknown_playbook_404() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-none/dispatch",
            Some(json!({"request_id": "missing"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(
        body["error"],
        json!("brain playbook not found: playbook-none")
    );
}

#[tokio::test]
async fn dispatch_invalid_request_id_400() {
    let h = Harness::new().await;
    seed_playbook(
        &h,
        &spec("playbook-bad", PlaybookTrigger::Manual {}, vec![agent("a")]),
    )
    .await;
    for bad in ["bad id!", &"x".repeat(49)] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/brain/playbooks/playbook-bad/dispatch",
                Some(json!({"request_id": bad})),
            )
            .await;
        assert_eq!(status, 400, "{body}");
    }
}
