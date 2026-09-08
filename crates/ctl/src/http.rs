//! Pure-function transport layer: a `RequestPlan` is plain data (method,
//! path, query, body) produced by per-domain planners; `send` executes it
//! with bearer auth. No classes, no hidden state.

use anyhow::{Context, Result};

use crate::ctx::Ctx;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPlan {
    pub method: reqwest::Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

impl RequestPlan {
    fn new(method: reqwest::Method, path: impl Into<String>) -> Self {
        RequestPlan {
            method,
            path: path.into(),
            query: Vec::new(),
            body: None,
        }
    }

    pub fn get(path: impl Into<String>) -> Self {
        Self::new(reqwest::Method::GET, path)
    }

    pub fn post(path: impl Into<String>) -> Self {
        Self::new(reqwest::Method::POST, path)
    }

    pub fn put(path: impl Into<String>) -> Self {
        Self::new(reqwest::Method::PUT, path)
    }

    pub fn patch(path: impl Into<String>) -> Self {
        Self::new(reqwest::Method::PATCH, path)
    }

    pub fn delete(path: impl Into<String>) -> Self {
        Self::new(reqwest::Method::DELETE, path)
    }

    /// Append a query pair.
    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.query.push((key.to_owned(), value.into()));
        self
    }

    /// Append a query pair only when the value is present.
    pub fn with_opt(self, key: &str, value: Option<String>) -> Self {
        match value {
            Some(value) => self.with(key, value),
            None => self,
        }
    }

    /// Attach a JSON body (POST/PUT/PATCH payloads).
    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        self.body = Some(body);
        self
    }

    pub fn with_opt_body(self, body: Option<serde_json::Value>) -> Self {
        match body {
            Some(body) => self.with_body(body),
            None => self,
        }
    }

    /// Fully-qualified URL for the given server base.
    pub fn url(&self, server: &str) -> String {
        let mut url = format!("{}{}", server, self.path);
        let mut first = true;
        for (key, value) in &self.query {
            url.push(if first { '?' } else { '&' });
            first = false;
            url.push_str(&format!("{}={}", urlencode(key), urlencode(value)));
        }
        url
    }
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Outcome of a completed HTTP call: status, raw body text and parsed JSON
/// (when the body is valid JSON).
pub struct Outcome {
    pub status: u16,
    pub text: String,
    pub json: Option<serde_json::Value>,
}

impl Outcome {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Error message: prefer the server's JSON `error` field, else raw text.
    pub fn error_message(&self) -> String {
        self.json
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(|e| e.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| self.text.trim().to_owned())
    }
}

/// Proxy-aware HTTP client (rustls; loopback bypasses proxies).
pub fn client() -> Result<reqwest::Client> {
    opencoder_core::net::build_http_client(None).context("build http client")
}

fn request(ctx: &Ctx, plan: &RequestPlan) -> reqwest::RequestBuilder {
    let builder = ctx
        .http
        .request(plan.method.clone(), plan.url(&ctx.server))
        .bearer_auth(&ctx.token)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    match &plan.body {
        Some(body) => builder.body(serde_json::to_vec(body).expect("serialize body")),
        None => builder,
    }
}

/// Execute a plan and buffer the response.
pub async fn send(ctx: &Ctx, plan: &RequestPlan) -> Result<Outcome> {
    let response = request(ctx, plan).send().await.context("send request")?;
    let status = response.status().as_u16();
    let text = response.text().await.context("read response body")?;
    let json = serde_json::from_str(&text).ok();
    Ok(Outcome { status, text, json })
}

/// Execute a plan without buffering: for SSE streams and artifact downloads.
pub async fn send_streaming(ctx: &Ctx, plan: &RequestPlan) -> Result<reqwest::Response> {
    request(ctx, plan)
        .send()
        .await
        .context("send streaming request")
}

/// Stream a successful response body to a file (`-` writes raw bytes to
/// stdout for piping). Non-2xx responses are converted into an error that
/// carries the server's JSON/text reason.
pub async fn save_body(response: reqwest::Response, dest: &std::path::Path) -> Result<()> {
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("HTTP {status}: {text}");
    }
    use futures::StreamExt;
    use std::io::Write;
    let mut stream = response.bytes_stream();
    let mut file: Box<dyn Write> = if dest == std::path::Path::new("-") {
        Box::new(std::io::stdout().lock())
    } else {
        Box::new(
            std::fs::File::create(dest)
                .with_context(|| format!("create output file {}", dest.display()))?,
        )
    };
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read artifact chunk")?;
        file.write_all(&chunk)
            .with_context(|| format!("write artifact to {}", dest.display()))?;
    }
    file.flush().ok();
    Ok(())
}

/// Exit code contract: 0 success, 1 transport, 2 auth, 4 server rejection.
pub fn exit_code(status: u16) -> i32 {
    match status {
        401 | 403 => 2,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_url_encodes_query() {
        let plan = RequestPlan::get("/api/executions")
            .with("kind", "agent")
            .with("cursor_id", "01ABC/ x");
        let url = plan.url("http://s:1");
        assert_eq!(
            url,
            "http://s:1/api/executions?kind=agent&cursor_id=01ABC%2F%20x"
        );
    }

    #[test]
    fn with_opt_skips_none_and_keeps_order() {
        let plan = RequestPlan::get("/x").with_opt("a", None).with("b", "2");
        assert_eq!(plan.query, vec![("b".to_owned(), "2".to_owned())]);
    }

    #[test]
    fn exit_code_classification() {
        assert_eq!(exit_code(401), 2);
        assert_eq!(exit_code(403), 2);
        assert_eq!(exit_code(404), 4);
        assert_eq!(exit_code(500), 4);
    }

    #[test]
    fn outcome_error_prefers_json_error_field() {
        let outcome = Outcome {
            status: 400,
            text: r#"{"ok":false,"error":"boom"}"#.into(),
            json: serde_json::from_str(r#"{"ok":false,"error":"boom"}"#).ok(),
        };
        assert_eq!(outcome.error_message(), "boom");
    }
}
