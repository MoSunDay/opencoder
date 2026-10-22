use super::*;
use opencoder_core::fleet::release::PlatformConfig;
use serde_json::{json, Value};
use std::{path::Path, process::Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::test]
async fn release_journal_waits_for_catalog_commit_and_old_writer_cannot_resume() {
    let root = tempfile::tempdir().unwrap();
    let lifecycle = Lifecycle::default();
    lifecycle
        .platform
        .set(PlatformConfig {
            release_id: "current".into(),
            state_dir: root.path().into(),
            host_service: "http://127.0.0.1:1".into(),
            resource_service: "http://127.0.0.1:1".into(),
        })
        .unwrap();
    lifecycle.listener_port.set(18001).unwrap();
    File::create(root.path().join("catalog-writer.lock")).unwrap();
    std::fs::write(
        root.path().join("release-state.json"),
        json!({
            "schema_version":1,"current":"current","phase":"complete",
            "releases":{"current":{"server_port":18001},"successor":{"server_port":18002}}
        })
        .to_string(),
    )
    .unwrap();
    let guard = writer(&lifecycle).await.unwrap();
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/platform");
    let mut transition = tokio::process::Command::new("python3")
        .args(["-c", "import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from rolling.state import Journal; j=Journal(Path(sys.argv[2])); j.data.update(current='successor',phase='switching'); print('saving',flush=True); j.save()"])
        .arg(scripts).arg(root.path()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(transition.stdout.take().unwrap())
        .read_line(&mut line)
        .await
        .unwrap();
    assert_eq!(line.trim(), "saving");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), transition.wait())
            .await
            .is_err()
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("release-state.json")).unwrap())
            .unwrap();
    assert_eq!(state["current"], "current");
    drop(guard);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), transition.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(writer(&lifecycle).await.is_err());
}
