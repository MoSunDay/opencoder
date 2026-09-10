mod chat_events;
mod chat_stream;
mod responses_stream;
mod transport;
mod usage;
#[cfg(test)]
use chat_events::{emit_delta, extract_reasoning, handle_event};
pub(crate) use usage::parse_usage as parse_response_usage;
#[cfg(test)]
use usage::parse_usage;

#[cfg(test)]
use crate::tool_call::ToolAccumulator;
use crate::{event::LlmEvent, request::ChatRequest, stream::ChatStream};
use anyhow::{anyhow, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct ChatClient {
    pub(crate) http: reqwest::Client,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) headers: Vec<(String, String)>,
    /// Event-level idle watchdog: if no decoded SSE event arrives within this
    /// window, the stream is treated as interrupted (and retried). Independent
    /// of the HTTP client's byte-level `read_timeout`, which catches total
    /// stalls; this catches a connection dribbling keep-alive heartbeats with
    /// no content.
    idle_timeout: Duration,
    config: Option<std::sync::Arc<opencoder_core::Config>>,
}

/// Default per-read idle timeout (10 minutes). A read that stalls for this
/// long without receiving any bytes is aborted; a stream that keeps
/// delivering data resets the timer on every chunk and is never interrupted.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(600);

impl ChatClient {
    /// Construct a client. `proxy` is an optional explicit proxy URL (e.g.
    /// `socks5://host:port`); when `None`, the proxy is resolved from
    /// `OPENCODER_PROXY` / `ALL_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY`. Loopback
    /// hosts always bypass the proxy.
    pub fn new(
        base_url: &str,
        api_key: &str,
        headers: &[(String, String)],
        proxy: Option<&str>,
    ) -> Result<Self> {
        Self::new_with_read_timeout(base_url, api_key, headers, DEFAULT_READ_TIMEOUT, proxy)
    }

    /// Construct a client with a custom per-read timeout. The same value
    /// governs both the HTTP client's byte-level `read_timeout` and the
    /// event-level idle watchdog, so a configured `stream_idle_timeout` flows
    /// through both layers consistently. See [`Self::new`] for proxy semantics.
    pub fn new_with_read_timeout(
        base_url: &str,
        api_key: &str,
        headers: &[(String, String)],
        read_timeout: Duration,
        proxy: Option<&str>,
    ) -> Result<Self> {
        let http = opencoder_core::net::build_http_client_with_read_timeout(proxy, read_timeout)?;
        Ok(ChatClient {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            headers: headers.to_vec(),
            idle_timeout: read_timeout,
            config: None,
        })
    }

    /// A configured client routes every request by its model's provider prefix.
    /// The supplied endpoint also selects the default embeddings route.
    pub fn from_config(
        config: &opencoder_core::Config,
        ep: &opencoder_core::Endpoint,
    ) -> Result<Self> {
        let mut client = Self::new_with_read_timeout(
            &ep.base_url,
            &ep.api_key,
            &ep.headers,
            config.stream_idle_timeout(),
            config.network.proxy.as_deref(),
        )?;
        client.config = Some(std::sync::Arc::new(config.clone()));
        Ok(client)
    }

    fn prepare(&self, mut req: ChatRequest) -> Result<PreparedRequest> {
        use opencoder_core::{ProviderProtocol, ProviderState};
        let mut ep = opencoder_core::Endpoint {
            provider: String::new(),
            protocol: ProviderProtocol::ChatCompletions,
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            headers: self.headers.clone(),
        };
        if let Some(config) = &self.config {
            let mut route = config.as_ref().clone();
            route.model = if req.model.contains('/') {
                req.model.clone()
            } else {
                format!("{}/{}", config.provider_id(), req.model)
            };
            if route.provider_id() != config.provider_id()
                && !config.providers.contains_key(route.provider_id())
            {
                return Err(anyhow!("unknown model provider `{}`", route.provider_id()));
            }
            ep = route.resolve_endpoint()?;
            req.model = route.model_id().to_owned();
        }
        let scope = ProviderState {
            provider: ep.provider.clone(),
            base_url: ep.base_url.trim_end_matches('/').to_owned(),
            model: req.model.clone(),
            output: Vec::new(),
        };
        let (path, body) = match ep.protocol {
            ProviderProtocol::ChatCompletions => ("chat/completions", req.to_body()),
            ProviderProtocol::Responses => (
                "responses",
                crate::responses::request::to_body(&req, &scope)?,
            ),
        };
        let url = format!("{}/{path}", scope.base_url);
        Ok(PreparedRequest {
            ep,
            scope,
            body,
            url,
        })
    }

    pub fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        use opencoder_core::ProviderProtocol;
        let PreparedRequest {
            ep,
            scope,
            body,
            url,
        } = self.prepare(req)?;
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        let client = self.http.clone();
        let idle_timeout = self.idle_timeout;
        tokio::spawn(async move {
            let result = match ep.protocol {
                ProviderProtocol::ChatCompletions => {
                    chat_stream::run_stream(
                        client,
                        url,
                        ep.api_key,
                        ep.headers,
                        body,
                        tx.clone(),
                        idle_timeout,
                    )
                    .await
                }
                ProviderProtocol::Responses => {
                    responses_stream::run(
                        responses_stream::Request {
                            http: client,
                            url,
                            key: ep.api_key,
                            headers: ep.headers,
                            body,
                            idle_timeout,
                        },
                        scope,
                        tx.clone(),
                    )
                    .await
                }
            };
            if let Err(e) = result {
                let _ = tx
                    .send(LlmEvent::Error(format!("stream failed: {e:#}")))
                    .await;
            }
        });
        Ok(rx)
    }
}

struct PreparedRequest {
    ep: opencoder_core::Endpoint,
    scope: opencoder_core::ProviderState,
    body: serde_json::Value,
    url: String,
}

impl ChatStream for ChatClient {
    fn request_body(&self, req: &ChatRequest) -> Result<serde_json::Value> {
        Ok(self.prepare(req.clone())?.body)
    }
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        ChatClient::chat_stream(self, req)
    }
    fn embed(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        crate::embed::embeddings_via(self, texts, model)
    }
}

/// Build the HTTP header map for a chat request. Built-in headers
/// (`authorization`, `content-type`, `accept`) are applied first; entries in
/// `custom` then override any built-in with the same (case-insensitive) name.
/// Malformed custom entries (invalid header name or value bytes) are silently
/// skipped so one bad entry can't break the whole stream. Pure and
/// side-effect-free so the override/merge behavior is unit-testable.
pub fn build_header_map(key: &str, custom: &[(String, String)]) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    // The Authorization header is mandatory. A key whose bytes are invalid in
    // an HTTP header value (e.g. a stray newline copied from env config) used
    // to be silently skipped, leaving the request unauthenticated and yielding
    // a confusing provider 401. Surface it as an explicit error instead.
    let auth = HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| anyhow!("api key contains bytes invalid in an HTTP header value"))?;
    map.insert(AUTHORIZATION, auth);
    map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    map.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    for (name, value) in custom {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.insert(n, v);
        }
    }
    Ok(map)
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
