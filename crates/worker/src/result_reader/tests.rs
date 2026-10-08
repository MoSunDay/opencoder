use super::*;
use std::io::{Seek, SeekFrom, Write};

fn saved(
    root: &std::path::Path,
    id: &str,
    kind: ExecutionKind,
    result: Value,
) -> (PathBuf, DetailFieldRequest) {
    let layout = DirectoryLayout::new(root.to_path_buf(), None).unwrap();
    let path = layout.record_path(kind, id).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    opencoder_core::atomic_write(&path, &serde_json::to_vec(&json!({
        "assignment":{"index":{"id":id,"kind":kind,"node_id":"node-reader","created_at":1,"status":"done"},
            "request":{"id":id,"kind":kind},"definition":{"name":"readers"}},"result":result,
        "queue":{"an_old_runtime_field":true},"events":[]
    })).unwrap()).unwrap();
    (
        path,
        DetailFieldRequest {
            execution: ExecutionRef {
                id: id.into(),
                kind,
            },
            field: "result".into(),
            offset: 0,
        },
    )
}

fn read(reader: &ResultReader, root: &std::path::Path, request: &DetailFieldRequest) -> RpcReply {
    reader
        .read_blocking(root.to_path_buf(), "node-reader".into(), request.clone())
        .unwrap()
}

#[test]
fn historical_result_is_read_without_loading_or_recovering_its_runtime() {
    let root = tempfile::tempdir().unwrap();
    let output = json!({"output_text":"历史证据".repeat(50_000)});
    let (path, mut request) = saved(
        root.path(),
        "agent-old",
        ExecutionKind::Agent,
        output.clone(),
    );
    let original = std::fs::read(&path).unwrap();
    let reader = ResultReader::default();
    let mut bytes = Vec::new();
    let mut version = None;
    loop {
        let reply = read(&reader, root.path(), &request);
        assert_eq!(reply.status, 200);
        let v = reply.body["version"].as_str().unwrap().to_string();
        assert_eq!(version.get_or_insert(v.clone()), &v);
        bytes.extend(
            base64::engine::general_purpose::STANDARD
                .decode(reply.body["bytes_b64"].as_str().unwrap())
                .unwrap(),
        );
        if reply.body["eof"] == true {
            break;
        }
        request.offset = reply.body["next_offset"].as_u64().unwrap();
    }
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), output);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(reader.1.load(std::sync::atomic::Ordering::Relaxed), 1);
    request.offset = 0;
    let restarted = ResultReader::default();
    assert_eq!(
        read(&restarted, root.path(), &request).body["version"],
        version.clone().unwrap()
    );
    saved(
        root.path(),
        "agent-old",
        ExecutionKind::Agent,
        json!({"output_text":"new turn"}),
    );
    let changed = read(&reader, root.path(), &request);
    assert_ne!(changed.body["version"], version.unwrap());
    assert_eq!(
        changed.body["total_bytes"],
        serde_json::to_vec(&json!({"output_text":"new turn"}))
            .unwrap()
            .len()
    );
    assert_eq!(reader.1.load(std::sync::atomic::Ordering::Relaxed), 2);
}

#[test]
fn team_64_mib_is_hashed_once_and_replacement_or_in_place_changes_invalidate_it() {
    let root = tempfile::tempdir().unwrap();
    let (_, mut request) = saved(root.path(), "team-large", ExecutionKind::Team, json!({}));
    request.field = "team.topic".into();
    let layout = DirectoryLayout::new(root.path().to_path_buf(), None).unwrap();
    let path = opencoder_team::layout::topic_file(
        &layout
            .team_state_dir(ExecutionKind::Team, "team-large")
            .unwrap(),
        "readers",
        "team-large",
    )
    .unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = File::create(&path).unwrap();
    file.write_all(b"{\"summary\":\"").unwrap();
    let block = [b'x'; 64 * 1024];
    for _ in 0..1024 {
        file.write_all(&block).unwrap();
    }
    file.write_all(b"\"}").unwrap();
    file.flush().unwrap();
    let reader = ResultReader::default();
    let version = read(&reader, root.path(), &request).body["version"].clone();
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    loop {
        let reply = read(&reader, root.path(), &request);
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["version"], version);
        digest.update(
            base64::engine::general_purpose::STANDARD
                .decode(reply.body["bytes_b64"].as_str().unwrap())
                .unwrap(),
        );
        if reply.body["eof"] == true {
            break;
        }
        request.offset = reply.body["next_offset"].as_u64().unwrap();
    }
    assert_eq!(
        format!("{:x}", digest.finalize()),
        version.as_str().unwrap()
    );
    assert_eq!(reader.1.load(std::sync::atomic::Ordering::Relaxed), 1);
    request.offset = 0;
    file.seek(SeekFrom::Start(20)).unwrap();
    file.write_all(b"y").unwrap();
    file.flush().unwrap();
    assert_ne!(
        read(&reader, root.path(), &request).body["version"],
        version
    );
    opencoder_core::atomic_write(&path, br#"{"summary":"short"}"#).unwrap();
    assert_eq!(read(&reader, root.path(), &request).body["total_bytes"], 19);
    assert_eq!(reader.1.load(std::sync::atomic::Ordering::Relaxed), 3);
}

#[test]
fn mismatched_owner_kind_and_symlink_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (path, request) = saved(root.path(), "agent-safe", ExecutionKind::Agent, json!({}));
    let reader = ResultReader::default();
    assert!(reader
        .read_blocking(root.path().into(), "another-node".into(), request.clone())
        .is_err());
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["assignment"]["index"]["kind"] = json!("team");
    opencoder_core::atomic_write(&path, &serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(reader
        .read_blocking(root.path().into(), "node-reader".into(), request.clone())
        .is_err());
    let mut wrong = request.clone();
    wrong.execution.id = "../outside".into();
    assert!(reader
        .read_blocking(root.path().into(), "node-reader".into(), wrong)
        .is_err());
    #[cfg(unix)]
    {
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(root.path().join("elsewhere"), &path).unwrap();
        assert!(reader
            .read_blocking(root.path().into(), "node-reader".into(), request)
            .is_err());
    }
}
