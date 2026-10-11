use crate::support::Harness;
use reqwest::Method;
use serde_json::json;

#[tokio::test]
async fn initiative_order_http_persists_checks_conflicts_and_requires_write_permission() {
    let h = Harness::new().await;
    let (_, project) = h
        .req(
            Method::POST,
            "/api/project/goals",
            Some(json!({"title":"order"})),
        )
        .await;
    let goal_id = project["id"].as_str().unwrap();
    let mut ids = Vec::new();
    for index in 0..3 {
        let (status, row) = h
            .req(
                Method::POST,
                "/api/project/initiatives",
                Some(json!({"title":format!("Item {index}"),"goal_id":goal_id,"sort":index})),
            )
            .await;
        assert_eq!(status, 200, "{row}");
        ids.push(row["id"].as_str().unwrap().to_owned());
    }
    let path = format!("/api/project/goals/{goal_id}/initiatives/order");
    let next = vec![ids[2].clone(), ids[0].clone(), ids[1].clone()];
    let body = json!({"expected_ids":ids,"ids":next});
    let (_, viewer) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"order-viewer","role":"viewer"})),
        )
        .await;
    assert_eq!(
        h.req_raw(
            Method::PUT,
            &path,
            Some(body.clone()),
            viewer["token"].as_str()
        )
        .await
        .status(),
        403
    );
    assert_eq!(h.req(Method::PUT, &path, Some(body.clone())).await.0, 200);
    assert_eq!(h.req(Method::PUT, &path, Some(body)).await.0, 409);
    assert_eq!(
        h.req(
            Method::PUT,
            &path,
            Some(json!({"expected_ids":next,"ids":[ids[0],ids[0],ids[1]]}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        h.req(
            Method::PUT,
            &path,
            Some(json!({"expected_ids":[ids[0]],"ids":[ids[0]]}))
        )
        .await
        .0,
        409
    );
    assert_eq!(
        h.req(
            Method::PUT,
            "/api/project/goals/missing/initiatives/order",
            Some(json!({"expected_ids":[],"ids":[]}))
        )
        .await
        .0,
        404
    );
    let (status, overview) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200);
    let goal = overview["goals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == goal_id)
        .unwrap();
    let saved: Vec<_> = goal["initiatives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(saved, next);
}
