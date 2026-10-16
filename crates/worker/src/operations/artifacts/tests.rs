use super::*;
use serde_json::json;

#[tokio::test]
async fn shared_artifacts_require_declarations_and_reject_changed_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("evidence.txt");
    std::fs::write(&path, b"baseline failed").unwrap();
    let request = ArtifactRequest {
        execution: ExecutionRef {
            id: "operator-evidence".into(),
            kind: ExecutionKind::Operator,
        },
        step: "workspace".into(),
        index: None,
        file: "evidence.txt".into(),
        offset: 0,
        version: None,
    };
    assert_eq!(
        read_directory(directory.path(), &request, true)
            .await
            .unwrap()
            .status,
        400
    );
    let digest = format!("{:x}", Sha256::digest(b"baseline failed"));
    std::fs::write(
        directory.path().join("artifacts.json"),
        json!({"files":[{
        "path":"evidence.txt","bytes":15,"sha256":digest}]})
        .to_string(),
    )
    .unwrap();
    let read = read_directory(directory.path(), &request, true)
        .await
        .unwrap();
    assert_eq!(read.status, 200, "{read:?}");
    assert_eq!(read.body["total_bytes"], 15);
    std::fs::write(&path, b"baseline passed").unwrap();
    assert_eq!(
        read_directory(directory.path(), &request, true)
            .await
            .unwrap()
            .status,
        409
    );
    let mut traversal = request.clone();
    traversal.file = "../evidence.txt".into();
    assert_eq!(
        read_directory(directory.path(), &traversal, true)
            .await
            .unwrap()
            .status,
        400
    );
    #[cfg(unix)]
    {
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", &path).unwrap();
        assert!(read_directory(directory.path(), &request, true)
            .await
            .is_err());
    }
}
