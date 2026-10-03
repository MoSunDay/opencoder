use super::*;

#[tokio::test]
async fn capability_field_requirements_persist_in_target_and_reach_the_scheduler_catalog() {
    let h = Harness::new().await;
    let id = seed_cap(&h, "verify an exact code revision").await;
    let path = format!("/api/brain/capabilities/{id}/target");
    let target = json!({"kind":"operator","target":"act",
        "required_inputs":["revision"],"required_outputs":["passed","failures"]});
    let (status, body) = h.req(Method::PUT, &path, Some(target.clone())).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["target"], target);
    let (status, library) = h.req(Method::GET, "/api/brain/library", None).await;
    assert_eq!(status, 200, "{library}");
    let cap = library["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|cap| cap["id"] == id)
        .unwrap();
    assert_eq!(cap["required_inputs"], target["required_inputs"]);
    assert_eq!(cap["required_outputs"], target["required_outputs"]);
    for field in ["required_inputs", "required_outputs"] {
        for invalid in [json!([""]), json!([" x"]), json!(["x", "x"])] {
            let mut body = target.clone();
            body[field] = invalid;
            let (status, body) = h.req(Method::PUT, &path, Some(body)).await;
            assert_eq!(status, 400, "{body}");
        }
    }
    let (_, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(
        body["target"], target,
        "invalid requirements must not overwrite the binding"
    );
}
