use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::post,
    Json, Router,
};
use axum::{http::StatusCode, response::IntoResponse};
use opencoder_core::{Config, Message, Role};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::{collections::VecDeque, convert::Infallible, sync::Arc};

pub struct ModelBridge {
    pub url: String,
    server: tokio::task::JoinHandle<()>,
}

fn request(value: Value) -> ChatRequest {
    let messages = value["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let text = row["content"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    row["content"]
                        .as_array()
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|part| part["text"].as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default()
                });
            let mut message = Message::user("fixture", text);
            message.role = match row["role"].as_str().unwrap() {
                "system" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            message
        })
        .collect();
    ChatRequest {
        purpose: RequestPurpose::Conversation,
        model: value["model"].as_str().unwrap().into(),
        messages,
        tools: value["tools"].as_array().cloned().unwrap_or_default(),
        tool_choice: None,
        temperature: value["temperature"].as_f64(),
        max_tokens: value["max_tokens"].as_u64(),
        reasoning_effort: value["reasoning_effort"].as_str().map(str::to_string),
        cache_salt: None,
    }
}

struct StreamState {
    receiver: tokio::sync::mpsc::Receiver<LlmEvent>,
    first: Option<LlmEvent>,
    queued: VecDeque<String>,
    text: String,
    tool_started: bool,
    done: bool,
}

fn frame(delta: Value, reason: Option<&str>) -> String {
    json!({"choices":[{"index":0,"delta":delta,"finish_reason":reason}]}).to_string()
}

impl ModelBridge {
    pub fn start(client: Arc<dyn ChatStream>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let app = Router::new().route("/v1/chat/completions", post(
            |State(client): State<Arc<dyn ChatStream>>, Json(value): Json<Value>| async move {
                let mut receiver = client.chat_stream(request(value)).unwrap();
                let first = receiver.recv().await;
                if first.is_none() {
                    return (StatusCode::BAD_REQUEST, Json(json!({"error":{"message":"stream ended without completion"}}))).into_response();
                }
                if let Some(LlmEvent::Error(message)) = first {
                    return (StatusCode::BAD_REQUEST, Json(json!({"error":{"message":message}}))).into_response();
                }
                let state = StreamState { receiver, first, queued: VecDeque::new(), text: String::new(), tool_started: false, done: false };
                let stream = futures::stream::unfold(state, |mut state| async move {
                    loop {
                        if let Some(data) = state.queued.pop_front() { return Some((Ok::<_, Infallible>(Event::default().data(data)), state)); }
                        if state.done { return None; }
                        let event = match state.first.take() { Some(event) => Some(event), None => state.receiver.recv().await };
                        match event {
                            Some(LlmEvent::TextDelta(text)) => {
                                state.text.push_str(&text);
                                state.queued.push_back(frame(json!({"content":text}), None));
                            },
                            Some(LlmEvent::ReasoningDelta(text)) => state.queued.push_back(frame(json!({"reasoning_content":text}), None)),
                            Some(LlmEvent::ToolCallStart { index, id, name }) => {
                                state.tool_started = true;
                                state.queued.push_back(frame(json!({"tool_calls":[{"index":index,"id":id,"type":"function","function":{"name":name,"arguments":""}}]}), None));
                            },
                            Some(LlmEvent::ToolCallDelta { index, arguments }) => state.queued.push_back(frame(json!({"tool_calls":[{"index":index,"function":{"arguments":arguments}}]}), None)),
                            Some(LlmEvent::Completed { text, tool_calls, .. }) => {
                                if state.text.is_empty() && !text.is_empty() { state.queued.push_back(frame(json!({"content":text}), None)); }
                                if !state.tool_started && !tool_calls.is_empty() {
                                    let calls: Vec<_> = tool_calls.iter().enumerate().map(|(index, call)| json!({"index":index,"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.input.to_string()}})).collect();
                                    state.queued.push_back(frame(json!({"tool_calls":calls}), None));
                                }
                                state.queued.push_back(frame(json!({}), Some(if tool_calls.is_empty() { "stop" } else { "tool_calls" })));
                                state.queued.push_back("[DONE]".into());
                                state.done = true;
                            },
                            Some(LlmEvent::Error(message)) => {
                                state.queued.push_back(json!({"error":{"message":message}}).to_string());
                                state.done = true;
                            },
                            None => { state.done = true; },
                            _ => {},
                        }
                    }
                });
                Sse::new(stream).into_response()
            }
        )).with_state(client);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, server }
    }

    pub fn configure(&self, config: &mut Config) {
        config.model = "fixture/model".into();
        config.providers.clear();
        config.provider.base_url = self.url.clone();
        config.provider.api_key = Some("local-fixture".into());
        config.provider.protocol = "chat_completions".into();
        config.local_memory = false;
    }
}

impl Drop for ModelBridge {
    fn drop(&mut self) {
        self.server.abort();
    }
}
