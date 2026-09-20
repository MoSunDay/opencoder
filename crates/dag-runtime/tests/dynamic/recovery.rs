use super::support::*;
use opencoder_dag::DagRunStatus;
use opencoder_dag_runtime::{execute_run, resume_run};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn resume_freezes_inputs_skips_success_and_never_appends_twice() {
    let mut f = Fixture::new().await;
    // A real file-agent resource: source content and version must remain unchanged.
    let pool = f.tmp.path().join("agents/prompts/probe");
    std::fs::create_dir_all(pool.join("v1")).unwrap();
    std::fs::write(pool.join("meta.json"), r#"{"current":1,"versions":[1]}"#).unwrap();
    std::fs::write(pool.join("v1/how.md"), "original-how\n").unwrap();
    let card = f.tmp.path().join("agents/probe");
    std::fs::create_dir_all(&card).unwrap();
    std::fs::write(card.join("meta.json"), r#"{"current":{"prompt":"probe"}}"#).unwrap();
    let mut step = dynamic_agent();
    step["kind"]["template"]["agent"] = json!("probe");
    let run = f.run(json!([step]), json!({"items":["DONE-ITEM","RETRY-ITEM"]}));
    let client = Scripted::new(|text| {
        if text.contains("RETRY-ITEM") {
            (Duration::from_millis(100), Err("retry me".into()))
        } else {
            (Duration::ZERO, Ok("{\"saved\":true}".into()))
        }
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(
        execute_run(f.deps(client), run.clone(), rx).await.unwrap(),
        DagRunStatus::Error
    );
    let how = f.text(&run, "process/instances/1/how.md");
    assert_eq!(
        std::fs::read_to_string(pool.join("v1/how.md")).unwrap(),
        "original-how\n"
    );
    assert_eq!(
        std::fs::read_to_string(pool.join("meta.json")).unwrap(),
        r#"{"current":1,"versions":[1]}"#
    );
    let saved_session = f.json(&run, "process/instances/0/session.json");
    std::fs::write(
        f.root.join(&run.run_id).join("input.json"),
        r#"{"items":["CHANGED"]}"#,
    )
    .unwrap();
    std::fs::write(pool.join("v1/how.md"), "edited after first attempt").unwrap();
    // Frozen resources survive even loss of the original source during recovery.
    f.config.agent.agents_dir = Some(f.tmp.path().join("unavailable"));
    let client = Scripted::new(|text| {
        assert!(text.contains("original-how") && text.contains("RETRY-ITEM"));
        assert!(!text.contains("CHANGED") && !text.contains("edited after"));
        (Duration::ZERO, Ok("{\"recovered\":true}".into()))
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    assert_eq!(
        resume_run(f.deps(client.clone()), run.clone(), rx)
            .await
            .unwrap(),
        DagRunStatus::Done
    );
    assert_eq!(client.requests.lock().unwrap().len(), 1);
    assert_eq!(f.text(&run, "process/instances/1/how.md"), how);
    assert_eq!(
        f.json(&run, "process/instances/0/session.json"),
        saved_session
    );
    assert_eq!(
        f.json(&run, "process/output.json"),
        json!([{"saved":true},{"recovered":true}])
    );
    assert_eq!(
        std::fs::read_to_string(pool.join("v1/how.md")).unwrap(),
        "edited after first attempt"
    );
    assert!(!pool.join("v2").exists());
    // Container loader consumes the same frozen agent + local how copy.
    let loaded = opencoder_dag_runtime::exec::how_copy::load(
        &f.root.join(&run.run_id).join("process/instances/1"),
    )
    .unwrap();
    assert!(loaded.prompt.contains("RETRY-ITEM"));
    assert_eq!(loaded.prompt.matches("RETRY-ITEM").count(), 1);
}

#[tokio::test]
async fn timeout_is_per_instance_and_user_cancel_is_run_cancellation() {
    let f = Fixture::new().await;
    let mut step = dynamic_agent();
    step["timeout_secs"] = json!(1);
    let run = f.run(json!([step]), json!({"items":["hang", "hang-2"]}));
    let client = Scripted::new(|_| (Duration::from_secs(10), Ok("{}".into())));
    let (_, rx) = tokio::sync::watch::channel(false);
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        execute_run(f.deps(client), run.clone(), rx),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(status, DagRunStatus::Error);
    assert!(f.json(&run, "process/meta.json")["error"]
        .as_str()
        .unwrap()
        .contains("timeout"));
    let run = f.run(json!([dynamic_agent()]), json!({"items":["hang","queued"]}));
    let client = Scripted::new(|_| (Duration::from_secs(10), Ok("{}".into())));
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(true).unwrap();
    });
    assert_eq!(
        execute_run(f.deps(client), run.clone(), rx).await.unwrap(),
        DagRunStatus::Cancelled
    );
}
