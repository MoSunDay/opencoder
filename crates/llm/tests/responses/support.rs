#![allow(dead_code)] // Shared with session/CLI integration targets, each using a different subset.
use opencoder_core::{Config, ProviderConfig};
use opencoder_llm::{ChatClient, ChatRequest, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
    pub chunk_size: usize,
    pub hold: Duration,
}
impl Reply {
    pub fn events(events: Vec<Value>) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            body: events
                .iter()
                .map(|v| {
                    format!(
                        "event: {}\r\ndata: {v}\r\n\r\n",
                        v["type"].as_str().unwrap_or("message")
                    )
                })
                .collect(),
            chunk_size: usize::MAX,
            hold: Duration::ZERO,
        }
    }
    pub fn json(body: Value) -> Self {
        Self {
            body: body.to_string(),
            content_type: "application/json",
            ..Self::events(vec![])
        }
    }
}

pub struct Server {
    pub url: String,
    pub requests: Arc<Mutex<Vec<(String, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(replies: Vec<Reply>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let task = tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            let header_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                raw.extend_from_slice(&buf[..n]);
                if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8(raw[..header_end].to_vec()).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap_or(0);
            while raw.len() < header_end + length {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            let body = serde_json::from_slice(&raw[header_end..header_end + length]).unwrap();
            captured.lock().unwrap().push((headers, body));
            let header = format!("HTTP/1.1 {} Test\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.status, reply.content_type, reply.body.len());
            if socket.write_all(header.as_bytes()).await.is_err() {
                continue;
            }
            for chunk in reply.body.as_bytes().chunks(reply.chunk_size) {
                if socket.write_all(chunk).await.is_err() {
                    break;
                }
            }
            tokio::time::sleep(reply.hold).await;
        }
    });
    Server {
        url,
        requests,
        task,
    }
}

pub fn config(url: &str) -> Config {
    Config {
        model: "fixture/gpt-6-astra".into(),
        provider: ProviderConfig {
            protocol: "responses".into(),
            base_url: url.into(),
            api_key: Some("fixture-key".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn client(config: &Config) -> ChatClient {
    ChatClient::from_config(config, &config.resolve_endpoint().unwrap()).unwrap()
}
pub fn request() -> ChatRequest {
    ChatRequest {
        purpose: RequestPurpose::Conversation,
        model: "fixture/gpt-6-astra".into(),
        messages: vec![opencoder_core::Message::user("u", "hello")],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: Some("high".into()),
        cache_salt: None,
    }
}
pub fn answer(text: &str) -> Value {
    json!({"type":"message", "id":"msg_1", "role":"assistant", "status":"completed", "phase":"final_answer",
        "content":[{"type":"output_text", "text":text, "annotations":[]}]})
}
pub fn reasoning() -> Value {
    json!({"type":"reasoning", "id":"rs_1", "encrypted_content":"opaque-fixture", "summary":[{"type":"summary_text", "text":"plan"}]})
}
pub fn call(id: &str, name: &str, arguments: Value) -> Value {
    json!({"type":"function_call", "id":format!("fc_{id}"), "call_id":id, "name":name, "arguments":arguments.to_string(), "status":"completed"})
}
pub fn response(output: Vec<Value>) -> Value {
    json!({"id":"resp_1", "object":"response", "status":"completed", "output":output,
        "usage":{"input_tokens":100, "output_tokens":30, "total_tokens":130,
            "input_tokens_details":{"cached_tokens":80,"cache_write_tokens":10},
            "output_tokens_details":{"reasoning_tokens":20}}})
}
pub fn completed(output: Vec<Value>) -> Value {
    json!({"type":"response.completed", "response":response(output)})
}
pub async fn collect(client: &ChatClient, req: ChatRequest) -> Vec<LlmEvent> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut rx = client.chat_stream(req).unwrap();
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    })
    .await
    .expect("stream did not terminate")
}
