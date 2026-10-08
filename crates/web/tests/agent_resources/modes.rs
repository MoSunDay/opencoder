use super::support::Server;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use opencoder_llm::MockChatClient;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn shared_resource_permissions_round_trip_and_invalid_bits_reject_before_write() {
    let server = Server::start(Arc::new(MockChatClient::new())).await;
    let body = |name, mode| {
        json!({"name":name,"files":[{
            "path":"program","content_b64":B64.encode(b"tool bytes"),"mode":mode
        }]})
    };
    let (status, value) = server
        .call(
            "POST",
            "/api/agents/resources/tools",
            Some(body("mode-test", 0o755)),
        )
        .await;
    assert_eq!(status, 200, "{value}");
    let (status, value) = server
        .call(
            "GET",
            "/api/agents/resources/tools/mode-test/versions/1/files/program",
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(value["mode"], 0o755);
    assert_eq!(value["content_b64"], B64.encode(b"tool bytes"));
    let (status, value) = server
        .call(
            "GET",
            "/api/agents/resources/tools/mode-test/versions/1",
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        value["files"],
        json!([{"path":"program","content_b64":B64.encode(b"tool bytes"),"mode":0o755}])
    );
    let (status, _) = server
        .call(
            "POST",
            "/api/agents/resources/tools",
            Some(body("invalid-mode", 0o4755)),
        )
        .await;
    assert_eq!(status, 400);
    assert!(!server.root.join("tools/invalid-mode").exists());
    for (version, expected) in [(0, 400), (2, 404)] {
        let route = format!("/api/agents/resources/tools/mode-test/versions/{version}");
        let (status, _) = server.call("GET", &route, None).await;
        assert_eq!(status, expected);
    }
}
