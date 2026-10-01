use crate::worker::{UiCmd, UiEvent};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use base64::Engine;
use opencoder_core::{
    fleet::{ExecutionKind, MessageChunk, MessageCursor, MessagePage},
    harness::{RemoteSession, ServerCapability},
    Config, Message,
};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Default)]
pub struct MockState {
    pub requests: Vec<(String, Value)>,
    pub events: Vec<Value>,
    pub messages: Vec<Message>,
    pub created: bool,
    pub fail_create_once: bool,
    pub auth_denied: bool,
    pub payload: Vec<u8>,
    pub questions: Vec<Value>,
}
pub struct Fixture {
    pub url: String,
    pub state: Arc<Mutex<MockState>>,
    server: tokio::task::JoinHandle<()>,
    pub store: Arc<dyn Store>,
    pub mock: Arc<MockChatClient>,
    pub workdir: tempfile::TempDir,
}
impl Fixture {
    pub async fn new() -> Self {
        let state = Arc::new(Mutex::new(MockState::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(handler).with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            state,
            server,
            store: Arc::new(LibsqlStore::open_memory().await.unwrap()),
            mock: Arc::new(MockChatClient::new()),
            workdir: tempfile::tempdir().unwrap(),
        }
    }
    pub fn config(&self) -> Config {
        let mut config = Config::default();
        config.opencoder_server.enabled = true;
        config.opencoder_server.url = self.url.clone();
        config
    }
    pub fn binding(&self, kind: ExecutionKind) -> RemoteSession {
        RemoteSession {
            server_url: self.url.clone(),
            capability: ServerCapability {
                id: "registered-codex".into(),
                kind,
                target: "codex".into(),
                summary: "Server registration".into(),
            },
            created: false,
            initial_input: None,
        }
    }
    pub async fn task(
        &self,
        kind: ExecutionKind,
    ) -> (
        String,
        mpsc::Sender<UiCmd>,
        mpsc::Receiver<UiEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let session = super::super::create(
            self.binding(kind),
            self.config(),
            self.mock.clone(),
            self.store.clone(),
            self.workdir.path(),
        )
        .await
        .unwrap();
        let id = session.id.clone();
        let (tx, rx) = mpsc::channel(32);
        let (ui, events) = mpsc::channel(512);
        let worker = crate::worker::spawn_task(session, rx, ui);
        (id, tx, events, worker)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn handler(State(state): State<Arc<Mutex<MockState>>>, request: Request<Body>) -> Response {
    let mut path = request.uri().path().to_owned();
    let mut query = request.uri().query().unwrap_or("").to_owned();
    let method = request.method().clone();
    let header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let input: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let mut state = state.lock().unwrap();
    state
        .requests
        .push((format!("{method} {path}"), input.clone()));
    if state.auth_denied {
        return (StatusCode::FORBIDDEN, axum::Json(json!({"error":"denied"}))).into_response();
    }
    if path == "/api/tui/agent-capabilities" {
        return axum::Json(json!({"capabilities":[{"id":"registered-codex","kind":"operator","target":"codex","summary":"Ops"},{"id":"bad-dag","kind":"dag","target":"dag","summary":"Dag"}],"authorization":header})).into_response();
    }
    if path == "/api/executions" {
        if state.fail_create_once {
            state.fail_create_once = false;
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(json!({"error":"try again"})),
            )
                .into_response();
        }
        state.created = true;
        return (StatusCode::ACCEPTED, axum::Json(json!({"accepted":true}))).into_response();
    }
    if path.ends_with("/commands")
        && input["input"]["tail"]
            .as_str()
            .is_some_and(|v| v.starts_with("transcript?"))
    {
        query = input["input"]["tail"]
            .as_str()
            .unwrap()
            .split_once('?')
            .unwrap()
            .1
            .into();
        path = "/transcript".into();
    }
    if path.ends_with("/commands") {
        if input["input"]["tail"] == "questions" {
            return axum::Json(json!({"questions":state.questions})).into_response();
        }
        return axum::Json(json!({"ok":true,"admitted_seq":42})).into_response();
    }
    if path.ends_with("/index") {
        let id = path.split('/').nth(3).unwrap();
        let kind = if id.starts_with("operator-") {
            "operator"
        } else {
            "agent"
        };
        return axum::Json(
            json!({"id":id,"kind":kind,"node_id":"auto-node","status":"running","created_at":1}),
        )
        .into_response();
    }
    let query_value = |key: &str| -> u64 {
        query
            .split('&')
            .find_map(|entry| {
                entry
                    .split_once('=')
                    .filter(|(k, _)| *k == key)
                    .and_then(|(_, v)| v.parse().ok())
            })
            .unwrap_or(0)
    };
    if path.ends_with("/events-page") {
        let after = query_value("after");
        return axum::Json(json!({"events":state.events.iter().filter(|e|e["seq"].as_u64().unwrap()>after).collect::<Vec<_>>(),"more":false,"finished":false})).into_response();
    }
    if path.ends_with("/events") {
        let after = query_value("after");
        let text = state
            .events
            .iter()
            .filter(|e| e["seq"].as_u64().unwrap() > after)
            .map(|e| {
                format!(
                    "id: {}\nevent: {}\ndata: {}\n\n",
                    e["seq"],
                    e["kind"].as_str().unwrap(),
                    e["data"]
                )
            })
            .collect::<String>()
            + "event: reconnect\ndata: release switch\n\n";
        return ([("content-type", "text/event-stream")], text).into_response();
    }
    if path.ends_with("/payload") {
        let offset = query_value("offset") as usize;
        let end = (offset + 4096).min(state.payload.len());
        return axum::Json(json!({"seq":7,"offset":offset,"next_offset":end,"total_bytes":state.payload.len(),"eof":end==state.payload.len(),"encoding":"utf8-base64","bytes_b64":base64::engine::general_purpose::STANDARD.encode(&state.payload[offset..end])})).into_response();
    }
    if path.ends_with("/transcript") {
        let cursor = MessageCursor {
            seq: query_value("seq") as i64,
            offset: query_value("offset"),
        };
        let mut chunks = Vec::new();
        let mut next_cursor = None;
        for (index, message) in state.messages.iter().enumerate() {
            let seq = (index + 1) as i64;
            if seq < cursor.seq || (seq == cursor.seq && cursor.offset == 0) {
                continue;
            }
            let bytes = serde_json::to_vec(message).unwrap();
            let offset = if seq == cursor.seq {
                cursor.offset as usize
            } else {
                0
            };
            let end = (offset + 4096).min(bytes.len());
            let eof = end == bytes.len();
            chunks.push(MessageChunk {
                seq,
                role: serde_json::to_value(message.role)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
                created_at: message.created_at,
                offset: offset as u64,
                next_offset: end as u64,
                total_bytes: bytes.len() as u64,
                eof,
                encoding: "base64".into(),
                bytes_b64: base64::engine::general_purpose::STANDARD.encode(&bytes[offset..end]),
            });
            if !eof {
                next_cursor = Some(MessageCursor {
                    seq,
                    offset: end as u64,
                });
                break;
            }
        }
        return axum::Json(MessagePage {
            chunks,
            more: next_cursor.is_some(),
            next_cursor,
        })
        .into_response();
    }
    (
        StatusCode::NOT_FOUND,
        axum::Json(json!({"error":"mock endpoint not found"})),
    )
        .into_response()
}

pub async fn until(
    events: &mut mpsc::Receiver<UiEvent>,
    predicate: impl Fn(&UiEvent) -> bool,
) -> Vec<UiEvent> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut result = Vec::new();
        loop {
            let event = events.recv().await.unwrap();
            let done = predicate(&event);
            result.push(event);
            if done {
                return result;
            }
        }
    })
    .await
    .expect("remote event timeout")
}
pub fn frame(seq: i64, event: opencoder_session::SessionEvent) -> Value {
    json!({"seq":seq,"kind":event.sse_kind(),"data":event.sse_data(),"ts":seq*10})
}
