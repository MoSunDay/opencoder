use super::*;
use std::io::{Read, Write};

fn opaque(bytes: &[u8]) -> Vec<u8> {
    let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
    out.extend(bytes);
    out.resize(out.len().next_multiple_of(4), 0);
    out
}
fn rpc(port: u16, procedure: u32, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    for value in [1u32, 0, 2, 100003, 3, procedure, 0, 0, 0, 0] {
        message.extend(value.to_be_bytes());
    }
    message.extend(body);
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(&(0x8000_0000u32 | message.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&message).unwrap();
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let mut result = vec![0; (u32::from_be_bytes(header) & 0x7fff_ffff) as usize];
    stream.read_exact(&mut result).unwrap();
    result
}
fn value(bytes: &[u8], pos: usize) -> u32 {
    u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap())
}

#[tokio::test]
async fn dedicated_service_exports_uploaded_bytes_and_rejects_nfs_writes() {
    let f = Fixture::new().await;
    assert_eq!(f.file("PUT", "note.txt", "handoff bytes").await.0, 201);
    let directory = f.list().await["directory"].as_str().unwrap().to_owned();
    let app = opencoder_control::release::todo_resources::build_app(
        f.state.workdir.clone(),
        f.root.path().join("data"),
        TOKEN.into(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config_path = f.state.workdir.join("opencoder.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    config["project"]["todo_storage"]["nfs_service_url"] =
        json!(format!("http://{}", listener.local_addr().unwrap()));
    std::fs::write(config_path, config.to_string()).unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/project/todos/nfs",
            Body::from("invalid json"),
            "wrong"
        )
        .await
        .0,
        401
    );
    assert_eq!(
        call(&app, "GET", "/api/dag/binaries", Body::empty(), TOKEN)
            .await
            .0,
        404
    );
    f.state
        .lifecycle
        .platform
        .set(opencoder_core::fleet::release::PlatformConfig {
            release_id: "todo-files-test".into(),
            state_dir: f.root.path().join("release-state"),
            host_service: "http://127.0.0.1:1".into(),
            resource_service: "http://127.0.0.1:1".into(),
        })
        .unwrap();
    let owner = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let app = &f.app;
    let (_, started) = json_call(
        app,
        "POST",
        "/api/project/todos/nfs",
        json!({"enabled":true}),
    )
    .await;
    assert_eq!(started["status"]["read_only"], true);
    let port = started["status"]["port"].as_u64().unwrap() as u16;
    tokio::task::spawn_blocking(move || {
        let mut handle = b"/".to_vec();
        for name in format!("{directory}/note.txt").split('/') {
            let mut request = opaque(&handle);
            request.extend(opaque(name.as_bytes()));
            let response = rpc(port, 3, &request);
            assert_eq!(value(&response, 24), 0);
            handle = response[32..32 + value(&response, 28) as usize].to_vec();
        }
        let mut request = opaque(&handle);
        request.extend(0u64.to_be_bytes());
        request.extend(100u32.to_be_bytes());
        let response = rpc(port, 6, &request);
        assert_eq!(value(&response, 24), 0);
        assert!(response.windows(13).any(|bytes| bytes == b"handoff bytes"));
        let mut request = opaque(&handle);
        request.extend(0u64.to_be_bytes());
        request.extend(1u32.to_be_bytes());
        request.extend(2u32.to_be_bytes());
        request.extend(opaque(b"x"));
        assert_eq!(value(&rpc(port, 7, &request), 24), 30); // NFS3ERR_ROFS.
    })
    .await
    .unwrap();
    let (_, stopped) = json_call(
        app,
        "POST",
        "/api/project/todos/nfs",
        json!({"enabled":false}),
    )
    .await;
    assert_eq!(stopped["status"]["running"], false);
    assert_eq!(f.file("GET", "note.txt", "").await.1, b"handoff bytes");
    owner.abort();
}
