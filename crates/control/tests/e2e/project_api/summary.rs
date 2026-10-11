use crate::support::Harness;
use reqwest::Method;
use serde_json::json;

#[tokio::test]
async fn project_and_initiative_summary_http_roundtrip_and_permissions() {
    let h = Harness::new().await;
    let (_, viewer) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"summary-viewer","role":"viewer"})),
        )
        .await;
    let token = viewer["token"].as_str().unwrap();
    for collection in ["goals", "initiatives"] {
        let path = format!("/api/project/{collection}");
        let mut body = json!({"title":"work","current_summary":" initial ","summary_updated_at":1});
        if collection == "initiatives" {
            assert_eq!(h.req(Method::POST, &path, Some(body.clone())).await.0, 422);
            let (status, listed) = h.req(Method::GET, &path, None).await;
            assert_eq!(status, 200);
            assert_eq!(listed[collection], json!([]));
            body.as_object_mut().unwrap().remove("summary_updated_at");
        }
        let (status, created) = h.req(Method::POST, &path, Some(body)).await;
        assert_eq!(status, 200, "{created}");
        assert_eq!(created["current_summary"], "initial");
        assert!(created["summary_updated_at"].as_i64().unwrap() > 1);
        let path = format!("{path}/{}", created["id"].as_str().unwrap());
        assert_eq!(
            h.req(Method::PATCH, &path, Some(json!({"title":"renamed"})))
                .await
                .0,
            200
        );
        let (_, changed) = h.req(Method::GET, &path, None).await;
        assert_eq!(changed["summary_updated_at"], created["summary_updated_at"]);
        assert_eq!(
            h.req_raw(
                Method::PATCH,
                &path,
                Some(json!({"current_summary":"denied"})),
                Some(token)
            )
            .await
            .status(),
            403
        );
        assert_eq!(
            h.req(
                Method::PATCH,
                &path,
                Some(json!({"current_summary":"cleared blockers"}))
            )
            .await
            .0,
            200
        );
        let (_, changed) = h.req(Method::GET, &path, None).await;
        assert_eq!(changed["current_summary"], "cleared blockers");
        assert_eq!(
            h.req(Method::PATCH, &path, Some(json!({"current_summary":""})))
                .await
                .0,
            200
        );
        assert_eq!(
            h.req(Method::GET, &path, None).await.1["current_summary"],
            ""
        );
    }
}

#[tokio::test]
async fn summary_history_is_bounded_readable_and_restorable_with_write_permission() {
    let h = Harness::new().await;
    let (_, viewer) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"history-viewer","role":"viewer"})),
        )
        .await;
    let token = viewer["token"].as_str().unwrap();
    for collection in ["goals", "initiatives"] {
        let (_, created) = h
            .req(
                Method::POST,
                &format!("/api/project/{collection}"),
                Some(json!({"title":"versions","current_summary":"initial"})),
            )
            .await;
        let path = format!(
            "/api/project/{collection}/{}",
            created["id"].as_str().unwrap()
        );
        let history_path = format!("{path}/summary-versions");
        for n in 1..=12 {
            assert_eq!(
                h.req(
                    Method::PATCH,
                    &path,
                    Some(json!({"current_summary":format!("summary {n}")}))
                )
                .await
                .0,
                200
            );
        }
        let reply = h
            .req_raw(Method::GET, &history_path, None, Some(token))
            .await;
        assert_eq!(reply.status(), 200);
        let history: serde_json::Value = reply.json().await.unwrap();
        assert_eq!(history["limit"], 10);
        assert_eq!(history["versions"].as_array().unwrap().len(), 10);
        assert_eq!(history["versions"][0]["version"], 13);
        let restore = format!("{history_path}/4/restore");
        assert_eq!(
            h.req_raw(Method::POST, &restore, None, Some(token))
                .await
                .status(),
            403
        );
        assert_eq!(
            h.req(Method::POST, &format!("{history_path}/3/restore"), None)
                .await
                .0,
            404
        );
        assert_eq!(
            h.req(Method::POST, &format!("{history_path}/0/restore"), None)
                .await
                .0,
            400
        );
        let (status, restored) = h.req(Method::POST, &restore, None).await;
        assert_eq!(status, 200, "{restored}");
        assert_eq!(restored["version"], 14);
        assert_eq!(restored["restored_from"], 4);
        assert_eq!(restored["current_summary"], "summary 3");
        let (_, current) = h.req(Method::GET, &path, None).await;
        assert_eq!(current["current_summary"], "summary 3");
        assert_eq!(current["summary_updated_at"], restored["created_at"]);
        let (_, history) = h.req(Method::GET, &history_path, None).await;
        assert_eq!(history["versions"].as_array().unwrap().len(), 10);
        assert_eq!(history["versions"][0], restored);
        assert_eq!(history["versions"][1]["current_summary"], "summary 12");
    }
    assert_eq!(
        h.req(
            Method::GET,
            "/api/project/goals/missing/summary-versions",
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        h.req(
            Method::GET,
            "/api/project/todos/missing/summary-versions",
            None
        )
        .await
        .0,
        404
    );
}
