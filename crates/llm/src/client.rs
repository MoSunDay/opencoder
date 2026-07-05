use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::warn;

use crate::event::{LlmEvent, Usage};
use crate::request::ChatRequest;
use crate::sse::SseDecoder;
use crate::stream::ChatStream;
use crate::tool_call::ToolAccumulator;

#[derive(Debug, Clone)]
pub struct ChatParams {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct ChatClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl ChatClient {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(1800))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .context("build http client")?;
        Ok(ChatClient {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        })
    }

    pub fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        let url = format!("{}/chat/completions", self.base_url);
        let body = req.to_body();
        let client = self.http.clone();
        let key = self.api_key.clone();

        tokio::spawn(async move {
            if let Err(e) = run_stream(client, url, key, body, tx.clone()).await {
                let _ = tx.send(LlmEvent::Error(format!("stream failed: {e:#}"))).await;
            }
        });
        Ok(rx)
    }
}

impl ChatStream for ChatClient {
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        ChatClient::chat_stream(self, req)
    }
    fn backend(&self) -> &'static str {
        "openai"
    }
}

async fn run_stream(
    client: reqwest::Client,
    url: String,
    key: String,
    body: Value,
    tx: mpsc::Sender<LlmEvent>,
) -> Result<()> {
    let resp = client
        .post(&url)
        .bearer_auth(&key)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .context("send chat request")?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("http {status}: {}", truncate(&text, 800)));
    }

    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut tools = ToolAccumulator::default();
    let mut usage: Option<Usage> = None;
    let mut finished = false;
    let mut text_buf = String::new();

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("read stream chunk")?;
        decoder.push(&bytes);
        for data in decoder.drain() {
            if tx.is_closed() {
                return Ok(());
            }
            let parsed = match crate::sse::parse_chunk(&data) {
                Some(v) => v,
                None => continue,
            };
            handle_event(&parsed, &mut tools, &mut usage, &mut finished, &mut text_buf, &tx).await?;
        }
    }
    for data in decoder.flush_remaining() {
        if let Some(parsed) = crate::sse::parse_chunk(&data) {
            handle_event(&parsed, &mut tools, &mut usage, &mut finished, &mut text_buf, &tx).await?;
        }
    }

    let tool_calls = tools.finish_all().unwrap_or_default();
    let _ = tx
        .send(LlmEvent::Completed {
            text: text_buf,
            tool_calls,
            usage,
        })
        .await;
    Ok(())
}

async fn handle_event(
    parsed: &Value,
    tools: &mut ToolAccumulator,
    usage: &mut Option<Usage>,
    finished: &mut bool,
    text_buf: &mut String,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<()> {
    if let Some(u) = parsed.get("usage") {
        *usage = Some(parse_usage(u));
    }
    let choices = match parsed.get("choices").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return Ok(()),
    };
    for choice in choices {
        if let Some(delta) = choice.get("delta") {
            emit_delta(delta, tools, text_buf, tx).await?;
        }
        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            *finished = true;
            if matches!(fr, "length" | "content_filter") {
                warn!(finish_reason = fr, "stream finished early");
            }
        }
    }
    Ok(())
}

async fn emit_delta(delta: &Value, tools: &mut ToolAccumulator, text_buf: &mut String, tx: &mpsc::Sender<LlmEvent>) -> Result<()> {
    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            text_buf.push_str(content);
            let _ = tx.send(LlmEvent::TextDelta(content.to_string())).await;
        }
    }
    if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            let _ = tx.send(LlmEvent::ReasoningDelta(reasoning.to_string())).await;
        }
    }
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let id = tc.get("id").and_then(|v| v.as_str());
            let name = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str());
            let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str());
            for ev in tools.apply(index, id, name, args) {
                let _ = tx.send(ev).await;
            }
        }
    }
    Ok(())
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or_default(),
        output_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or_default(),
        total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or_default(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}...", &s[..n]) }
}
