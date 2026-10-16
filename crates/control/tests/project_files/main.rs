//! Real control/router storage, role permissions, publication and NFS tests.
mod nfs;
mod service;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "todo-files-test";
struct Fixture {
    root: tempfile::TempDir,
    state: Arc<opencoder_control::AppState>,
    app: Router,
    id: String,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(
            work.join("opencoder.json"),
            json!({"project":{"todo_storage":{
            "files_dir":root.path().join("files"),"max_file_bytes":4*1024*1024,"nfs":{"port":0}}}})
            .to_string(),
        )
        .unwrap();
        let state = opencoder_control::new_state(work, root.path().join("data"), None)
            .await
            .unwrap();
        let app = opencoder_control::build_app(state.clone(), Some(TOKEN.into()), false);
        let (_, value) = json_call(
            &app,
            "POST",
            "/api/project/todos",
            json!({"title":"Task","draft":"handoff"}),
        )
        .await;
        let id = value["id"].as_str().unwrap().to_owned();
        Self {
            root,
            state,
            app,
            id,
        }
    }
    fn base(&self) -> String {
        format!("/api/project/todos/{}", self.id)
    }
    async fn file(&self, method: &str, path: &str, bytes: impl Into<Body>) -> (u16, Vec<u8>) {
        let query: String = path
            .as_bytes()
            .iter()
            .map(|b| format!("%{b:02X}"))
            .collect();
        call(
            &self.app,
            method,
            &format!("{}/file?path={query}", self.base()),
            bytes.into(),
            TOKEN,
        )
        .await
    }
    async fn list(&self) -> Value {
        let (status, value) = json_call(
            &self.app,
            "GET",
            &format!("{}/files", self.base()),
            Value::Null,
        )
        .await;
        assert_eq!(status, 200, "{value}");
        value
    }
}
async fn call(app: &Router, method: &str, path: &str, body: Body, token: &str) -> (u16, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    (
        status,
        to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
}
async fn json_call(app: &Router, method: &str, path: &str, value: Value) -> (u16, Value) {
    let (status, body) = call(app, method, path, Body::from(value.to_string()), TOKEN).await;
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn files_roundtrip_old_todo_date_restart_and_delete_guard() {
    let mut f = Fixture::new().await;
    let mut old = f.state.projects.get_todo(&f.id).await.unwrap().unwrap();
    old.id = "pt-old".into();
    old.created_at = 1_767_225_599_999; // 2025-12-31 UTC.
    f.state.projects.create_todo(&old).await.unwrap();
    f.id = old.id;
    let bytes = vec![0xab; 3 * 1024 * 1024];
    assert_eq!(f.file("PUT", "日志/a b.bin", bytes.clone()).await.0, 201);
    assert_eq!(f.file("PUT", "日志/a b.bin", "replacement").await.0, 409);
    let list = f.list().await;
    assert_eq!(list["directory"], "2025-12-31/pt-old");
    assert_eq!(list["files"][0]["path"], "日志/a b.bin");
    assert_eq!(list["files"][0]["bytes"], bytes.len());
    let (_, task) = json_call(&f.app, "GET", &f.base(), Value::Null).await;
    assert_eq!(task["draft"], "handoff");
    assert_eq!(
        json_call(&f.app, "DELETE", &f.base(), Value::Null).await.0,
        409
    );
    let data = f.root.path().join("data");
    #[cfg(unix)]
    let data = {
        let link = f.root.path().join("redirected-server-data");
        std::os::unix::fs::symlink(&data, &link).unwrap();
        link
    };
    let state = opencoder_control::new_state(f.state.workdir.clone(), data, None)
        .await
        .unwrap();
    f.app = opencoder_control::build_app(state, Some(TOKEN.into()), false);
    assert_eq!(f.file("GET", "日志/a b.bin", "").await.1, bytes);
    assert_eq!(f.file("DELETE", "日志/a b.bin", "").await.0, 200);
    assert!(f.list().await["files"].as_array().unwrap().is_empty());
    assert_eq!(
        json_call(&f.app, "DELETE", &f.base(), Value::Null).await.0,
        200
    );
    assert_eq!(f.file("PUT", "later", "data").await.0, 404);
}

#[tokio::test]
async fn invalid_paths_size_interruptions_and_permissions_do_not_publish_files() {
    let f = Fixture::new().await;
    for path in ["../escape", "/absolute", "a//b", "a/./b", "a\\b", ""] {
        assert_eq!(f.file("PUT", path, "bad").await.0, 400, "{path}");
    }
    assert_eq!(
        f.file("PUT", "large", vec![0; 4 * 1024 * 1024 + 1]).await.0,
        413
    );
    let body = Body::from_stream(futures::stream::iter([
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"partial")),
        Err(std::io::Error::other("disconnected")),
    ]));
    assert_eq!(f.file("PUT", "interrupted", body).await.0, 400);
    assert!(f.list().await["files"].as_array().unwrap().is_empty());
    assert!(!std::fs::read_dir(f.root.path().join("files.state"))
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("upload-")));
    assert_eq!(f.file("PUT", "empty", "").await.0, 201);
    let (_, user) = json_call(
        &f.app,
        "POST",
        "/api/users",
        json!({"name":"reader","role":"viewer"}),
    )
    .await;
    let token = user["token"].as_str().unwrap();
    let path = format!("{}/file?path=empty", f.base());
    assert_eq!(
        call(&f.app, "GET", &path, Body::empty(), token).await.0,
        200
    );
    for method in ["PUT", "DELETE"] {
        assert_eq!(
            call(&f.app, method, &path, Body::empty(), token).await.0,
            403
        );
    }
    assert_eq!(
        call(&f.app, "GET", &path, Body::empty(), "invalid").await.0,
        401
    );
    let (_, user) = json_call(
        &f.app,
        "POST",
        "/api/users",
        json!({"name":"writer","role":"editor"}),
    )
    .await;
    assert_eq!(
        call(
            &f.app,
            "PUT",
            &format!("{}/file?path=editor", f.base()),
            Body::from("data"),
            user["token"].as_str().unwrap()
        )
        .await
        .0,
        201
    );
}

#[tokio::test]
async fn concurrent_uploads_and_deletion_share_a_filesystem_lock() {
    let f = Fixture::new().await;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(2);
    let app = f.app.clone();
    let url = format!("{}/file?path=slow", f.base());
    let upload = tokio::spawn(async move {
        call(
            &app,
            "PUT",
            &url,
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            TOKEN,
        )
        .await
    });
    tx.send(Ok(axum::body::Bytes::from_static(b"data")))
        .await
        .unwrap();
    // Wait until the streaming handler has acquired its persistent lock.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if std::fs::read_dir(f.root.path().join("files.state"))
                .unwrap()
                .any(|v| {
                    v.unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("upload-")
                })
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Another state instance models overlapping Server processes during rollout.
    let state =
        opencoder_control::new_state(f.state.workdir.clone(), f.root.path().join("data"), None)
            .await
            .unwrap();
    let app = opencoder_control::build_app(state, Some(TOKEN.into()), false);
    let base = f.base();
    let delete = tokio::spawn(async move { json_call(&app, "DELETE", &base, Value::Null).await });
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    assert!(!delete.is_finished());
    drop(tx);
    assert_eq!(upload.await.unwrap().0, 201);
    assert_eq!(delete.await.unwrap().0, 409);
    let (a, b) = tokio::join!(f.file("PUT", "same", "a"), f.file("PUT", "same", "b"));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [201, 409]);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_special_files_and_config_rebinding_are_rejected() {
    let f = Fixture::new().await;
    let directory = f
        .root
        .path()
        .join("files")
        .join(f.list().await["directory"].as_str().unwrap());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(f.root.path().join("outside"), "private").unwrap();
    std::os::unix::fs::symlink(f.root.path().join("outside"), directory.join("link")).unwrap();
    assert_eq!(f.file("GET", "link", "").await.0, 500);
    assert_eq!(
        json_call(&f.app, "DELETE", &f.base(), Value::Null).await.0,
        500
    );
    std::fs::remove_file(directory.join("link")).unwrap();
    let socket = std::os::unix::net::UnixListener::bind(directory.join("socket")).unwrap();
    assert_eq!(f.file("GET", "socket", "").await.0, 500);
    assert_eq!(
        json_call(&f.app, "DELETE", &f.base(), Value::Null).await.0,
        500
    );
    drop(socket);
    std::fs::remove_file(directory.join("socket")).unwrap();
    std::fs::write(
        f.state.workdir.join("opencoder.json"),
        json!({"project":{"todo_storage":{"files_dir":f.root.path().join("new-files")}}})
            .to_string(),
    )
    .unwrap();
    assert_eq!(f.file("PUT", "new", "data").await.0, 500);
    assert!(!f.root.path().join("new-files").exists());
}

#[tokio::test]
async fn missing_bound_storage_is_an_error_instead_of_an_empty_list() {
    let f = Fixture::new().await;
    std::fs::remove_dir(f.root.path().join("files")).unwrap();
    assert_eq!(
        json_call(&f.app, "GET", &format!("{}/files", f.base()), Value::Null)
            .await
            .0,
        500
    );
    assert!(!f.root.path().join("files").exists());
    assert_eq!(
        json_call(&f.app, "DELETE", &f.base(), Value::Null).await.0,
        500
    );
}
