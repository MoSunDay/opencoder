use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use opencoder_core::identity::{Identity, Role};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(app: &Router, method: &str, path: &str, role: Role) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(Identity {
        name: "fixture".into(),
        role,
    });
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn tui_catalog_reads_library_agent_operator_with_user_role_without_exposing_definitions() {
    let root = tempfile::tempdir().unwrap();
    let workdir = root.path().join("work");
    let agents = root.path().join("agents");
    std::fs::create_dir_all(agents.join("codexops")).unwrap();
    std::fs::create_dir_all(&workdir).unwrap();
    let _scope = opencoder_core::scoped_config_home(root.path().join("home"));
    std::fs::write(
        workdir.join("opencoder.json"),
        json!({"agent":{"agents_dir":agents}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        agents.join("codexops/meta.json"),
        json!({"name":"codexops","harness":"codex","current":{"prompt":"codexops"}}).to_string(),
    )
    .unwrap();
    let prompt = agents.join("prompts/codexops");
    std::fs::create_dir_all(prompt.join("v1")).unwrap();
    std::fs::write(prompt.join("meta.json"), r#"{"current":1}"#).unwrap();
    std::fs::write(
        prompt.join("v1/soul.md"),
        "Use the registered Codex wrapper.",
    )
    .unwrap();
    let state = opencoder_control::new_state(workdir, root.path().join("state"), None)
        .await
        .unwrap();
    state.fleet.put_definition("brain_capability","ops",&json!({"id":"ops","kind":"operator","target":"codexops","summary":"Registered Ops","definition":{"private":"do not expose"}})).await.unwrap();
    state
        .fleet
        .put_definition(
            "brain_capability",
            "dag",
            &json!({"id":"dag","kind":"dag","target":"dag","summary":"Dag"}),
        )
        .await
        .unwrap();
    let app = opencoder_control::build_app(state, None, false);
    for role in [Role::Viewer, Role::Editor, Role::Admin] {
        let (status, body) = call(&app, "GET", "/api/tui/agent-capabilities", role).await;
        assert_eq!(status, 200, "{body}");
        let cards = body["capabilities"].as_array().unwrap();
        assert!(cards.iter().any(|card| card["id"] == "builtin-agent-act"));
        assert!(cards.iter().any(|card| card["id"] == "builtin-operator"));
        assert!(cards
            .iter()
            .any(|card| card["id"] == "ops" && card["target"] == "codexops"));
        assert!(!cards.iter().any(|card| card["kind"] == "dag"));
        assert!(cards
            .iter()
            .all(|card| card.as_object().unwrap().len() == 4));
        assert!(!body.to_string().contains("do not expose"));
    }
    assert_eq!(
        call(&app, "POST", "/api/tui/agent-capabilities", Role::Viewer)
            .await
            .0,
        403
    );
    assert_eq!(
        call(&app, "GET", "/api/brain/library", Role::Viewer)
            .await
            .0,
        200
    );
}
