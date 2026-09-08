use axum::{body::Body, http::Request, Router};
use base64::Engine;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn api(app: &Router, method: &str, path: &str, value: Value) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn custom_agent_publication_uses_configured_root_without_cross_server_leaks() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = Vec::new();
    for name in ["first", "second"] {
        let workdir = dir.path().join(name);
        let root = workdir.join("published");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(
            workdir.join("opencoder.json"),
            json!({"agent":{"agents_dir":root}}).to_string(),
        )
        .unwrap();
        let state = opencoder_control::new_state(workdir.clone(), workdir.join("state"), None)
            .await
            .unwrap();
        apps.push((opencoder_control::build_app(state, None, false), root));
    }
    async fn publish(app: &Router, name: &str) {
        api(app, "POST", "/api/agents/resources/prompts", json!({
            "name":name,"files":[{"path":"soul.md","content_b64":base64::engine::general_purpose::STANDARD.encode(name)}]
        })).await;
        api(
            app,
            "POST",
            "/api/agents",
            json!({"name":name,"current":{"prompt":name}}),
        )
        .await;
    }
    tokio::join!(
        publish(&apps[0].0, "agent-first"),
        publish(&apps[1].0, "agent-second")
    );
    for (index, name, other) in [
        (0, "agent-first", "agent-second"),
        (1, "agent-second", "agent-first"),
    ] {
        let (app, root) = &apps[index];
        assert!(root.join(name).join("meta.json").is_file());
        assert!(!root.join(other).exists());
        assert_eq!(
            std::fs::read_to_string(root.join("prompts").join(name).join("v1/soul.md")).unwrap(),
            name
        );
        let meta = api(app, "GET", &format!("/api/agents/{name}/meta"), Value::Null).await;
        assert!(meta.to_string().contains(name));
        assert!(!meta.to_string().contains(other));
    }
}
