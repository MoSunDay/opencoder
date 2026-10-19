use super::support::{binary, spec, Harness};
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;

fn default_root(h: &Harness) -> PathBuf {
    let work = h.dir.path().join("work");
    std::fs::write(
        work.join("opencoder.json"),
        json!({"dag":{"binary_dir":h.dir.path().join("binaries")}}).to_string(),
    )
    .unwrap();
    assert!(opencoder_core::Config::load(&work)
        .unwrap()
        .agent
        .agents_dir
        .is_none());
    let root = h.dir.path().join(".opencoder/agents");
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[tokio::test]
async fn default_agent_root_protects_dag_and_binary_until_reference_is_removed() {
    let h = Harness::new().await;
    let root = default_root(&h);
    assert_eq!(
        h.call("POST", "/api/dag/defs", spec("workflow")).await.0,
        200
    );
    assert_eq!(
        h.call("POST", "/api/dag/binaries", binary("tool", "fixture"))
            .await
            .0,
        201
    );
    let content = "/mnt/binaries/tool/v1/binary.bin dag-workflow";
    let body = json!({"name":"helper","files":[{"path":"runner.cjs",
        "content_b64":base64::engine::general_purpose::STANDARD.encode(content)}]});
    assert_eq!(
        h.call("POST", "/api/agents/resources/tools", body).await.0,
        200
    );
    let file = root.join("tools/helper/v1/runner.cjs");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), content);
    let endpoints = ["/api/dag/defs/workflow", "/api/dag/binaries/tool"];
    for endpoint in endpoints {
        let response = h.call("DELETE", endpoint, Value::Null).await;
        assert_eq!(response.0, 409, "{}", response.1);
        assert_eq!(response.1["code"], "catalog_in_use");
        assert_eq!(h.call("GET", endpoint, Value::Null).await.0, 200);
    }
    assert!(file.is_file());
    assert_eq!(
        h.call("DELETE", "/api/agents/resources/tools/helper", Value::Null)
            .await
            .0,
        200
    );
    for endpoint in endpoints {
        assert_eq!(h.call("DELETE", endpoint, Value::Null).await.0, 200);
        assert_eq!(h.call("GET", endpoint, Value::Null).await.0, 404);
    }
}

#[tokio::test]
async fn unavailable_default_agent_root_and_unreadable_files_block_deletion() {
    let h = Harness::new().await;
    let root = default_root(&h);
    assert_eq!(
        h.call("POST", "/api/dag/defs", spec("workflow")).await.0,
        200
    );
    std::fs::remove_dir(&root).unwrap();
    let endpoint = "/api/dag/defs/workflow";
    assert_eq!(h.call("DELETE", endpoint, Value::Null).await.0, 503);
    assert_eq!(h.call("GET", endpoint, Value::Null).await.0, 200);
    std::fs::write(&root, "not a directory").unwrap();
    assert_eq!(h.call("DELETE", endpoint, Value::Null).await.0, 503);
    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("opaque"), [0xff, 0]).unwrap();
    assert_eq!(h.call("DELETE", endpoint, Value::Null).await.0, 409);
    assert_eq!(h.call("GET", endpoint, Value::Null).await.0, 200);
}
