use super::support::{binary, spec, Harness};
use base64::Engine;
use serde_json::{json, Value};

#[tokio::test]
async fn every_legal_tool_extension_protects_referenced_pools_and_definitions() {
    let h = Harness::new().await;
    assert_eq!(
        h.call("GET", "/api/health", Value::Null).await.1["catalog_guard"],
        2
    );
    assert_eq!(
        h.call("POST", "/api/dag/binaries", binary("tool", "bytes"))
            .await
            .0,
        201
    );
    assert_eq!(
        h.call("POST", "/api/dag/defs", spec("workflow")).await.0,
        200
    );
    for file in [
        "runner.cjs",
        "runner.bash",
        "runner.txt",
        "runner.custom",
        "runner",
    ] {
        let content = "/mnt/binaries/tool/v1/binary.bin dag-workflow";
        let body = json!({"name":"helper","files":[{"path":file,"content_b64":base64::engine::general_purpose::STANDARD.encode(content)}]});
        assert_eq!(
            h.call("POST", "/api/agents/resources/tools", body).await.0,
            200
        );
        for endpoint in ["/api/dag/binaries/tool", "/api/dag/defs/workflow"] {
            let response = h.call("DELETE", endpoint, Value::Null).await;
            assert_eq!(response.0, 409, "{file}: {}", response.1);
            assert_eq!(response.1["code"], "catalog_in_use");
            assert_eq!(h.call("GET", endpoint, Value::Null).await.0, 200);
        }
        // Isolate each extension; these are disposable fixture resources.
        assert_eq!(
            h.call("DELETE", "/api/agents/resources/tools/helper", Value::Null)
                .await
                .0,
            200
        );
    }
    assert_eq!(
        h.call("DELETE", "/api/dag/binaries/tool", Value::Null)
            .await
            .0,
        200
    );
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/workflow", Value::Null)
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn unreadable_or_unverified_agent_data_never_allows_catalog_deletion() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("workflow")).await;
    let root = h.dir.path().join("agents");
    let path = root.join("unknown.custom");
    for bytes in [
        vec![0xff, 0xfe],
        b"text\0binary".to_vec(),
        vec![b'x'; 4 * 1024 * 1024 + 1],
    ] {
        std::fs::write(&path, bytes).unwrap();
        let response = h
            .call("DELETE", "/api/dag/defs/workflow", Value::Null)
            .await;
        assert_eq!(response.0, 409, "{}", response.1);
        assert!(response.1.to_string().contains("unverified-agent-file"));
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(&root).unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/workflow", Value::Null)
            .await
            .0,
        503
    );
    assert_eq!(
        h.call("GET", "/api/dag/defs/workflow", Value::Null).await.0,
        200
    );
}
