use super::*;

#[tokio::test]
async fn guarded_update_returns_conflict_without_overwriting_and_legacy_still_works() {
    let h = Harness::new().await;
    let id = seed_cap(&h, "original").await;
    let path = format!("/api/brain/capabilities/{id}");
    let (_, original) = h.req(Method::GET, &path, None).await;
    let mut update = capability_payload("accepted");
    update["expected_content_version"] = original["content_version"].clone();
    assert_eq!(h.req(Method::PUT, &path, Some(update.clone())).await.0, 200);
    update["summary"] = json!("stale overwrite");
    let (status, body) = h.req(Method::PUT, &path, Some(update)).await;
    assert_eq!(status, 409, "{body}");
    let (_, kept) = h.req(Method::GET, &path, None).await;
    assert_eq!(kept["capability"]["summary"], "accepted");
    assert_ne!(kept["content_version"], original["content_version"]);
    assert_eq!(
        h.req(
            Method::PUT,
            &path,
            Some(capability_payload("legacy update"))
        )
        .await
        .0,
        200
    );
}
