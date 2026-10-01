use anyhow::{bail, Context, Result};
use opencoder_core::{fleet::ExecutionKind, harness::ServerCapability, Config};
use reqwest::{Client, Method, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct ServerClient {
    client: Client,
    url: String,
    token: Option<String>,
}

pub fn normalize_url(value: &str) -> Result<String> {
    let url =
        reqwest::Url::parse(value.trim()).context("opencoder_server.url must be an HTTP(S) URL")?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "opencoder_server.url must be an HTTP(S) URL"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Server URL must not contain credentials, a query or a fragment"
    );
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

impl ServerClient {
    pub fn configured(config: &Config) -> Result<Self> {
        anyhow::ensure!(
            config.opencoder_server.enabled,
            "OpenCoder Server is disabled; enable opencoder_server.enabled in config"
        );
        Self::new(
            &config.opencoder_server.url,
            config.network.proxy.as_deref(),
        )
    }

    pub fn new(url: &str, proxy: Option<&str>) -> Result<Self> {
        Self::with_token(url, proxy, std::env::var("OPENCODER_SERVER_TOKEN").ok())
    }

    pub fn with_token(url: &str, proxy: Option<&str>, token: Option<String>) -> Result<Self> {
        Ok(Self {
            client: opencoder_core::net::build_http_client_with_read_timeout(
                proxy,
                Duration::from_secs(30),
            )?,
            url: normalize_url(url)?,
            token: token.filter(|token| !token.trim().is_empty()),
        })
    }

    pub async fn response(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        stream: bool,
    ) -> Result<Response> {
        let mut request = self.client.request(method, format!("{}{path}", self.url));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if !stream {
            request = request.timeout(Duration::from_secs(30));
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .context("cannot connect to OpenCoder Server")?;
        if response.status().is_success() {
            return Ok(response);
        }
        match response.status().as_u16() {
            401 => bail!("Server authentication failed; set OPENCODER_SERVER_TOKEN"),
            403 => bail!("Server denied this capability or operation for the current token"),
            status => {
                let detail: Value = response.json().await.unwrap_or_default();
                bail!(
                    "Server HTTP {status}: {}",
                    detail["error"].as_str().unwrap_or("request failed")
                );
            }
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self
            .response(Method::GET, path, None, false)
            .await?
            .json()
            .await?)
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        Ok(self
            .response(Method::POST, path, Some(body), false)
            .await?
            .json()
            .await?)
    }

    pub async fn command(&self, id: &str, action: &str, input: Value) -> Result<Value> {
        self.post(
            &format!("/api/executions/{id}/commands"),
            &serde_json::json!({"action":action,"input":input}),
        )
        .await
    }

    pub async fn capabilities(&self) -> Result<Vec<ServerCapability>> {
        #[derive(serde::Deserialize)]
        struct Catalog {
            capabilities: Vec<ServerCapability>,
        }
        let catalog: Catalog = self.get("/api/tui/agent-capabilities").await?;
        Ok(catalog
            .capabilities
            .into_iter()
            .filter(|card| {
                matches!(card.kind, ExecutionKind::Agent | ExecutionKind::Operator)
                    && opencoder_core::fleet::valid_id(&card.id)
                    && !card.target.is_empty()
                    && card.id != "self"
            })
            .collect())
    }
}
