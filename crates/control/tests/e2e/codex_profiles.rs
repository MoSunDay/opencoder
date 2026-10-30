//! Registered harness profile list/save roundtrip over real HTTP.
use crate::support::Harness;
use reqwest::Method;
use serde_json::{json, Value};

fn item<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("profile {name} missing from {body}"))
}

#[tokio::test]
async fn profiles_list_roundtrip_and_empty_start() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/harnesses/codex/profiles", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["items"], json!([]));

    for name in ["alpha", "beta"] {
        let (status, body) = h
            .req(
                Method::PUT,
                &format!("/api/harnesses/codex/profiles/{name}"),
                Some(json!({"model":"fixture-model"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["name"], name);
        assert_eq!(body["revision"], 1);
    }

    let (status, list) = h.req(Method::GET, "/api/harnesses/codex/profiles", None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    for name in ["alpha", "beta"] {
        let row = item(&list, name);
        assert_eq!(row["revision"], 1);
        assert_eq!(row["settings"]["model"], "fixture-model");
    }

    // Saving an existing profile bumps its revision instead of duplicating it.
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/harnesses/codex/profiles/alpha",
            Some(json!({"model":"fixture-model-updated"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["revision"], 2);
    let (status, list) = h.req(Method::GET, "/api/harnesses/codex/profiles", None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    let alpha = item(&list, "alpha");
    assert_eq!(alpha["revision"], 2);
    assert_eq!(alpha["settings"]["model"], "fixture-model-updated");
    assert_eq!(item(&list, "beta")["revision"], 1);
}
