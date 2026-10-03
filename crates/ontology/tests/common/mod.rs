#![allow(dead_code)] // Shared integration fixtures use different subsets in each suite.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use opencoder_core::identity::{Identity, Role};
use opencoder_ontology::AppState;
use serde_json::{json, Value};
use tower::ServiceExt;

pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub state: AppState,
    pub router: Router,
}
impl Fixture {
    pub async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::open(
            &directory.path().join("ontology.db"),
            &directory.path().join("files"),
        )
        .await
        .unwrap();
        let router = opencoder_ontology::router(state.clone());
        Self {
            directory,
            state,
            router,
        }
    }
    pub async fn call(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        call(&self.router, Role::Admin, method, path, body).await
    }
    pub async fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, value) = self.call(method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }
    pub async fn kind(&self) -> String {
        self.ok(
            "POST",
            "/envs/debug/entity-types",
            json!({"key":"service","name":"服务"}),
        )
        .await["item"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    pub async fn entity(&self, kind: &str, title: &str) -> Value {
        self.ok("POST", "/envs/debug/entities", creation(kind, title))
            .await["item"]
            .clone()
    }
    pub async fn connection(&self) -> libsql::Connection {
        libsql::Builder::new_local(self.directory.path().join("ontology.db"))
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap()
    }
}
pub fn creation(kind: &str, title: &str) -> Value {
    json!({"request_id":uuid::Uuid::new_v4(),"entity_type_id":kind,"name":title,"source":"# 来源","ext":"# 拓展","attributes":{}})
}
pub async fn call(
    router: &Router,
    role: Role,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    request.extensions_mut().insert(Identity {
        name: "tester".into(),
        role,
    });
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
