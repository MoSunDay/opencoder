use super::*;

/// GET /api/schedules lists the STORED definitions (created via the admin
/// API), echoes every field, stamps created_at/updated_at, computes next_run
/// only for parseable crons, and reports the file ops knob `scan_interval_secs`.
#[tokio::test]
async fn lists_definitions_fail_soft() {
    let h = Harness::new().await;
    write_schedules(&h, &json!({"schedules": [], "scan_interval_secs": 7}));
    let (status, body) = create_schedule(
        &h,
        json!({
            "id": "pinger", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act",
            "params": {"prompt": "daily {{now-1d:%Y-%m-%d}}"}
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("pinger"));

    // Parked definitions skip the cron check at the door (enable flips it
    // back on), so an unparseable cron still stores and lists inert.
    let (status, body) = create_schedule(
        &h,
        json!({
            "id": "parked", "cron": "not a cron", "kind": "agent", "target": "act",
            "enabled": false
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 2);
    let pinger = schedules.iter().find(|s| s["id"] == "pinger").unwrap();
    assert_eq!(pinger["kind"], json!("agent"));
    assert_eq!(pinger["enabled"], json!(true));
    assert_eq!(
        pinger["params"]["prompt"],
        json!("daily {{now-1d:%Y-%m-%d}}")
    );
    assert!(pinger["next_run"].as_i64().unwrap() > 0, "{}", pinger);
    assert!(pinger["created_at"].as_i64().unwrap() > 0, "{}", pinger);
    assert!(pinger["updated_at"].as_i64().unwrap() > 0, "{}", pinger);
    let parked = schedules.iter().find(|s| s["id"] == "parked").unwrap();
    assert_eq!(parked["enabled"], json!(false));
    assert!(
        parked["next_run"].is_null(),
        "bad cron → no next tick: {parked}"
    );
    assert_eq!(body["scan_interval_secs"], json!(7));
}

/// Full CRUD surface: create (auto id), duplicate → 409, invalid → 400,
/// PUT full update (created_at preserved), PATCH enable/disable, DELETE.
#[tokio::test]
async fn schedule_crud_round_trip() {
    let h = Harness::new().await;

    // Create with an explicit id.
    let (status, body) = create_schedule(
        &h,
        json!({"id": "crud_job", "cron": USER_AGENT_CRON, "kind": "agent",
               "target": "act", "params": {"prompt": "v1"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("crud_job"));

    // Duplicate id → 409 (updates go through PUT).
    let (status, body) = create_schedule(
        &h,
        json!({"id": "crud_job", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act"}),
    )
    .await;
    assert_eq!(status, 409, "{body}");

    // A missing id gets a generated one.
    let (status, body) = create_schedule(
        &h,
        json!({"cron": USER_AGENT_CRON, "kind": "agent", "target": "act"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let auto_id = body["id"].as_str().unwrap();
    assert!(auto_id.starts_with("schedule-"), "{body}");

    // Invalid bodies are rejected at the door: empty target, unknown id
    // characters, an enabled job with a broken cron, brain params missing
    // the objective contract.
    for invalid in [
        json!({"id": "no_target", "cron": USER_AGENT_CRON, "kind": "agent"}),
        json!({"id": "has.dot", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act"}),
        json!({"id": "bad_cron", "cron": "not a cron", "kind": "agent", "target": "act"}),
        json!({"id": "brain_no_obj", "cron": USER_AGENT_CRON, "kind": "brain",
               "target": "review_plan", "params": {}}),
    ] {
        let (status, body) = create_schedule(&h, invalid.clone()).await;
        assert_eq!(status, 400, "{invalid}: {body}");
    }

    let created_at = h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "crud_job")
        .unwrap()["created_at"]
        .as_i64()
        .unwrap();

    // PUT full update: cron + params change, created_at stays.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let (status, body) = h
        .req(
            reqwest::Method::PUT,
            "/api/schedules/crud_job",
            Some(
                json!({"cron": "* * * * *", "kind": "agent", "target": "act",
                        "params": {"prompt": "v2"}}),
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let job = h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "crud_job")
        .unwrap()
        .clone();
    assert_eq!(job["cron"], json!("* * * * *"));
    assert_eq!(job["params"]["prompt"], json!("v2"));
    assert_eq!(job["created_at"].as_i64(), Some(created_at));
    assert!(job["updated_at"].as_i64().unwrap() >= created_at);

    // PUT on an unknown id → 404 (create goes through POST).
    let (status, body) = h
        .req(
            reqwest::Method::PUT,
            "/api/schedules/ghost",
            Some(json!({"cron": USER_AGENT_CRON, "kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // PATCH enable/disable only; an unknown id → 404.
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/crud_job",
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let job = h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "crud_job")
        .unwrap()
        .clone();
    assert_eq!(job["enabled"], json!(false));
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/ghost",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Enabling a parked-with-broken-cron definition fails at the door.
    create_schedule(
        &h,
        json!({"id": "parked_bad", "cron": "not a cron", "kind": "agent",
               "target": "act", "enabled": false}),
    )
    .await;
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/parked_bad",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // DELETE removes the definition; the fire ledger stays queryable.
    let (status, body) = h
        .req(reqwest::Method::DELETE, "/api/schedules/crud_job", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(reqwest::Method::DELETE, "/api/schedules/ghost", None)
        .await;
    assert_eq!(status, 404, "{body}");
    let listed = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    assert!(
        !listed["schedules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == "crud_job"),
        "{listed}"
    );
    let (status, body) = h
        .req(reqwest::Method::GET, "/api/schedules/crud_job/runs", None)
        .await;
    assert_eq!(status, 200, "history survives the delete: {body}");
}

/// The legacy `schedules.json` degrades to a one-time seed: an empty table
/// imports every valid entry (invalid ones warn + skip, the old fail-soft
/// contract), and once definitions exist a mutated file is dead weight — a
/// restart must never resurrect a deleted or stale definition.
#[tokio::test]
async fn file_seed_is_one_time_and_table_gated() {
    let h = Harness::new().await;
    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["schedules"].as_array().unwrap().is_empty(),
        "no file, no definitions: {body}"
    );

    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "seeded", "cron": USER_AGENT_CRON, "kind": "agent",
                 "target": "act", "params": {"prompt": "ping"}},
                {"id": "broken", "cron": "not a cron", "kind": "agent", "target": "act"}
            ],
            "scan_interval_secs": 1
        }),
    );
    opencoder_control::seed_schedules::seed_schedules(&h.state.store, &h.state.workdir).await;
    let body = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 1, "invalid entries are skipped: {body}");
    assert_eq!(schedules[0]["id"], json!("seeded"));
    assert_eq!(schedules[0]["cron"], json!(USER_AGENT_CRON));
    assert!(schedules[0]["created_at"].as_i64().unwrap() > 0);
    assert_eq!(body["scan_interval_secs"], json!(1));

    // A mutated file does not re-import: the table is no longer empty.
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "seeded", "cron": "* * * * *", "kind": "agent",
                 "target": "renamed", "params": {"prompt": "changed"}},
                {"id": "latecomer", "cron": USER_AGENT_CRON, "kind": "agent",
                 "target": "act", "params": {"prompt": "new"}}
            ],
            "scan_interval_secs": 1
        }),
    );
    opencoder_control::seed_schedules::seed_schedules(&h.state.store, &h.state.workdir).await;
    let body = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 1, "one-time import, no merge: {body}");
    assert_eq!(schedules[0]["cron"], json!(USER_AGENT_CRON), "{body}");
    assert_eq!(schedules[0]["target"], json!("act"), "{body}");
}

/// The schedules surface (reads AND writes) is admin-only (unknown paths
/// default to closed).
#[tokio::test]
async fn schedule_apis_are_admin_only() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            reqwest::Method::POST,
            "/api/users",
            Some(json!({"name": "alice", "role": "user"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap().to_string();

    let resp = h
        .req_raw(reqwest::Method::GET, "/api/schedules", None, Some(&token))
        .await;
    assert_eq!(resp.status().as_u16(), 403);
    let resp = h
        .req_raw(
            reqwest::Method::GET,
            "/api/schedules/whatever/runs",
            None,
            Some(&token),
        )
        .await;
    assert_eq!(resp.status().as_u16(), 403);

    // Write paths are gated the same way.
    let body = json!({"id": "evil", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act"});
    for (method, path, payload) in [
        (reqwest::Method::POST, "/api/schedules", Some(body.clone())),
        (
            reqwest::Method::PUT,
            "/api/schedules/evil",
            Some(body.clone()),
        ),
        (
            reqwest::Method::PATCH,
            "/api/schedules/evil",
            Some(json!({"enabled": false})),
        ),
        (reqwest::Method::DELETE, "/api/schedules/evil", None),
        (reqwest::Method::POST, "/api/schedules/evil/run", None),
    ] {
        let resp = h.req_raw(method, path, payload, Some(&token)).await;
        assert_eq!(resp.status().as_u16(), 403, "{path}");
    }
}

/// Malformed schedule ids are rejected before hitting the store.
#[tokio::test]
async fn runs_rejects_invalid_ids() {
    let h = Harness::new().await;
    let too_long = "x".repeat(41);
    for id in ["has.dot", "has%20space", too_long.as_str()] {
        let (status, body) = h
            .req(
                reqwest::Method::GET,
                &format!("/api/schedules/{id}/runs"),
                None,
            )
            .await;
        assert_eq!(status, 400, "{id}: {body}");
    }
}
