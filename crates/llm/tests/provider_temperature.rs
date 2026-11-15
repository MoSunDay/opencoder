//! Wire-level tests for provider-pinned sampling temperature.
//!
//! Backends that only accept a single fixed temperature (e.g. reasoning
//! models locked to `1`) reject any other pinned value with HTTP 400, so
//! production call sites send none (provider default applies). A provider
//! configured with `temperature` pins it on every request purpose,
//! overriding whatever the call site asked for (a call site may still set
//! its own value — e.g. a future purpose default); providers without a pin
//! must pass call-site values through untouched.

use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatRequest, LlmEvent, RequestPurpose};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn req(model: &str, temperature: Option<f64>) -> ChatRequest {
    ChatRequest {
        purpose: RequestPurpose::Compaction,
        model: model.to_string(),
        messages: vec![opencoder_core::Message::user("u", "summarize")],
        tools: Vec::new(),
        tool_choice: None,
        temperature,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

/// Config with one provider (`pinned`, temperature 1.0) and one without a
/// pin (`free`), both pointing at the local mock server.
fn test_config(base_url: String) -> Config {
    let mut config = Config::default();
    config.providers.insert(
        "pinned".into(),
        opencoder_core::ProviderConfig {
            base_url: base_url.clone(),
            api_key: Some("sk-test".into()),
            temperature: Some(1.0),
            ..Default::default()
        },
    );
    config.providers.insert(
        "free".into(),
        opencoder_core::ProviderConfig {
            base_url: base_url.clone(),
            api_key: Some("sk-test".into()),
            ..Default::default()
        },
    );
    config
}

/// Serve one request: capture its JSON body, reply with a complete SSE
/// stream. Returns the captured body.
async fn serve_one_capture(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let body = read_request_body(&mut stream).await;
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
               Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    stream
        .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n")
        .await
        .unwrap();
    stream
        .write_all(
            b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        )
        .await
        .unwrap();
    stream.flush().await.unwrap();
    body
}

/// Read headers + Content-Length-delimited body of one HTTP request.
async fn read_request_body(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        assert!(n > 0, "client closed before sending a full request");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let len: usize = headers
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < header_end + len {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        assert!(n > 0, "client closed mid-body");
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf[header_end..header_end + len]).to_string()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn drain(mut rx: tokio::sync::mpsc::Receiver<LlmEvent>) {
    while rx.recv().await.is_some() {}
}

/// Run one request against a fresh single-shot server; return the body it saw.
async fn capture_body(config: &Config, model: &str, temperature: Option<f64>) -> serde_json::Value {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    // Repoint both providers at this ephemeral listener.
    let mut config = config.clone();
    let addr = listener.local_addr().unwrap().to_string();
    let base = format!("http://{addr}");
    for p in config.providers.values_mut() {
        p.base_url = base.clone();
    }
    config.model = model.to_string(); // route endpoint resolution at the test provider
    let served = serve_one_capture(listener);
    let ep = config.resolve_endpoint().unwrap();
    let client = ChatClient::from_config(&config, &ep).unwrap();
    let rx = client.chat_stream(req(model, temperature)).unwrap();
    let (body, _) = tokio::join!(served, drain(rx));
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn pinned_temperature_overrides_per_purpose_default() {
    let config = test_config("http://ignored".into());
    // A call-site value (any future purpose default) must be replaced by
    // the pinned 1.0.
    let body = capture_body(&config, "pinned/m", Some(0.2)).await;
    assert_eq!(body["temperature"], serde_json::json!(1.0));
}

#[tokio::test]
async fn pinned_temperature_applies_when_call_site_omits_it() {
    let config = test_config("http://ignored".into());
    // Conversation calls send no temperature; the pin still applies.
    let body = capture_body(&config, "pinned/m", None).await;
    assert_eq!(body["temperature"], serde_json::json!(1.0));
}

#[tokio::test]
async fn unpinned_provider_passes_call_site_value_through() {
    let config = test_config("http://ignored".into());
    let body = capture_body(&config, "free/m", Some(0.2)).await;
    assert_eq!(body["temperature"], serde_json::json!(0.2));
}

#[tokio::test]
async fn unpinned_provider_omits_temperature_when_call_site_omits_it() {
    let config = test_config("http://ignored".into());
    let body = capture_body(&config, "free/m", None).await;
    assert!(body.get("temperature").is_none());
}
