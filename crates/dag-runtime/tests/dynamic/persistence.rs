use super::support::*;
use opencoder_dag::DagRunStatus;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn scheduling_persistence_failure_cancels_and_drains_live_instances() {
    let f = Fixture::new().await;
    let run = f.run(json!([dynamic_agent()]), json!({"items":["FAST", "SLOW"]}));
    let progress = f.root.join(&run.run_id).join("process/progress.json");
    let client = Scripted::new(move |text| {
        if text.contains("FAST") {
            let progress = progress.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                std::fs::remove_file(&progress).unwrap();
                std::fs::create_dir(&progress).unwrap();
            });
            (Duration::from_millis(150), Ok("{}".into()))
        } else {
            (Duration::from_secs(30), Ok("{}".into()))
        }
    });
    let (_, rx) = tokio::sync::watch::channel(false);
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        opencoder_dag_runtime::execute_run(f.deps(client), run.clone(), rx),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(status, DagRunStatus::Error);
    assert_eq!(
        f.json(&run, "process/instances/1/meta.json")["outcome"],
        "cancelled"
    );
}
