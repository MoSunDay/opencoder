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
use opencoder_brain::playbook::{PlaybookRoute, PlaybookRouteKind};

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
                        route: None,
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
                    route: None,
                },
            )],
        ),
    )
    .await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-bound/dispatch",
            Some(json!({"request_id": "bindreq", "situation": "resize now"})),
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
            Some(json!({"request_id": "unbreq", "situation": "resize now"})),
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
    // "bad id!" fails the charset; 27 and 48 chars fail the verbatim-id
    // budget (26 = a full ULID). The 26-char boundary is covered by
    // `dispatch_rejects_oversized_request_id_400`.
    for bad in ["bad id!", &"x".repeat(27), &"x".repeat(48)] {
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

/// A 26-char request_id (the full execution-id key budget, same length as
/// the ULID fallback) is accepted — the oversized-cap 400 must not swallow
/// it; 27 chars is rejected with the collision-free message.
#[tokio::test]
async fn dispatch_rejects_oversized_request_id_400() {
    let h = Harness::new().await;
    // Every helper-built prompt references {situation}, so the D4 gate
    // answers for the accepted key — proving the key itself passed.
    seed_playbook(
        &h,
        &spec("playbook-keycap", PlaybookTrigger::Manual {}, vec![agent("a")]),
    )
    .await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-keycap/dispatch",
            Some(json!({"request_id": "x".repeat(26)})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("situation must not be empty when a step prompt references {situation}")
    );

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-keycap/dispatch",
            Some(json!({"request_id": "x".repeat(26), "situation": "fits"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert!(body["executions"][0]["id"]
        .as_str()
        .unwrap()
        .starts_with(&format!("agent-pbk-{}-a", "x".repeat(26))));

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-keycap/dispatch",
            Some(json!({"request_id": "x".repeat(27), "situation": "fits"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("request_id must be at most 26 chars to keep step execution ids collision-free")
    );
}

/// An empty situation only 400s when some step prompt actually substitutes
/// `{situation}`; placeholder-free playbooks dispatch fine without one.
#[tokio::test]
async fn dispatch_requires_situation_for_placeholder_steps() {
    let h = Harness::new().await;
    let mut echo = agent("echo");
    echo.prompt = "{situation}".into();
    seed_playbook(
        &h,
        &spec("playbook-situ", PlaybookTrigger::Manual {}, vec![echo]),
    )
    .await;
    let mut fixed = agent("fixed");
    fixed.prompt = "static prompt, no placeholder".into();
    seed_playbook(
        &h,
        &spec(
            "playbook-situ-free",
            PlaybookTrigger::Manual {},
            vec![fixed],
        ),
    )
    .await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-situ/dispatch",
            Some(json!({"request_id": "situ1"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("situation must not be empty when a step prompt references {situation}")
    );

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-situ/dispatch",
            Some(json!({"request_id": "situ2", "situation": "nightly run"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-situ-free/dispatch",
            Some(json!({"request_id": "situfree"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
}

/// A request_id reused with different dispatch content is a 409, not a
/// silent replay of the earlier executions; the same content replays 202.
#[tokio::test]
async fn dispatch_request_id_reuse_with_different_content_409() {
    let h = Harness::new().await;
    seed_playbook(
        &h,
        &spec("playbook-reuse", PlaybookTrigger::Manual {}, vec![agent("a")]),
    )
    .await;

    let (status, first) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-reuse/dispatch",
            Some(json!({"request_id": "reused1", "situation": "s1"})),
        )
        .await;
    assert_eq!(status, 202, "{first}");

    // Idempotent retry: same request_id, same content → same executions.
    let (status, replay) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-reuse/dispatch",
            Some(json!({"request_id": "reused1", "situation": "s1"})),
        )
        .await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay["executions"], first["executions"]);

    // Same request_id, different situation → 409 (the old executions must
    // not be presented as this dispatch's result).
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-reuse/dispatch",
            Some(json!({"request_id": "reused1", "situation": "s2"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("request_id already dispatched different content; use a new request_id")
    );
}

/// A brain step carrying an inline route resolves to that executor even
/// when the environment binds the capability elsewhere — the inline route
/// is the cross-end determinism channel.
#[tokio::test]
async fn dispatch_inline_brain_route_wins_over_binding() {
    let h = Harness::new().await;
    seed_dag(&h).await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/capabilities",
            Some(json!({
                "capability_type": "tool-usage",
                "summary": "route with a pinned executor",
                "input_desc": "a work request",
                "output_desc": "completed work",
                "eng_inputs": ["exemplar input"],
            })),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let capability = body["capability"]["id"].as_str().unwrap().to_string();

    // Conflicting environment binding: the default agent, not the dag.
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{capability}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    seed_playbook(
        &h,
        &spec(
            "playbook-route",
            PlaybookTrigger::Manual {},
            vec![step(
                "run",
                &[],
                PlaybookTarget::Brain {
                    capability_id: capability.clone(),
                    route: Some(PlaybookRoute {
                        kind: PlaybookRouteKind::Dag,
                        ref_: "pbk-dag".into(),
                    }),
                },
            )],
        ),
    )
    .await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-route/dispatch",
            Some(json!({"request_id": "routereq", "situation": "route me"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let run = &body["executions"][0];
    assert_eq!(run["kind"], json!("dag"), "{body}");
    assert!(run["id"]
        .as_str()
        .unwrap()
        .starts_with("dag-pbk-routereq-run"));
    assert_eq!(
        index_kind(&h, run["id"].as_str().unwrap()).await,
        Some(ExecutionKind::Dag)
    );
}
