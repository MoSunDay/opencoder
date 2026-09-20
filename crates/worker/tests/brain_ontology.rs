//! Legacy v2 brain writes are rejected while the historical surface remains
//! available for read-only migration. V3 behavior is covered by
//! `brain_scheduler_v3`.
#[path = "support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use serde_json::json;
use support::*;

#[tokio::test]
async fn legacy_v2_run_creation_is_read_only() {
    let (_config, _home) = isolated_config();
    let fleet = Fleet::new(1, mock()).await;
    for body in [
        json!({"id":"brain-v2-fixed","mode":"fixed","objective":"Review","plan":{"id":"plan","version":1}}),
        json!({"id":"brain-v2-dynamic","mode":"dynamic","objective":"Review","inputs":{}}),
        json!({"id":"brain-v2-input","mode":"fixed","objective":"Input","inputs":{}}),
    ] {
        let reply = fleet.call("POST", "/api/brain/runs", body).await;
        assert_eq!(reply.status, 409, "{reply:?}");
        assert!(reply.body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("schema_version: 3"));
    }
    assert!(fleet
        .state
        .fleet
        .indexes(None, Some(ExecutionKind::Brain), 100)
        .await
        .unwrap()
        .is_empty());
    fleet.shutdown().await;
}

#[tokio::test]
async fn v3_run_creation_requires_explicit_schema_version() {
    let (_config, _home) = isolated_config();
    let fleet = Fleet::new(1, mock()).await;
    let reply = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({"schema_version":2,"objective":"legacy"}),
        )
        .await;
    assert_eq!(reply.status, 409, "{reply:?}");
    assert!(reply.body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("read-only"));
    fleet.shutdown().await;
}
