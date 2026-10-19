use crate::AppState;
use anyhow::{Context, Result};
use axum::{body::Body, http::HeaderValue, response::Response};
use serde_json::Value;
use std::path::PathBuf;

pub(super) fn path(prefix: &str, name: &str) -> String {
    let mut url = reqwest::Url::parse("http://127.0.0.1/").expect("constant URL");
    url.set_path(prefix);
    url.path_segments_mut()
        .expect("HTTP path")
        .pop_if_empty()
        .push(name);
    url.path().to_owned()
}

pub(super) struct Pools {
    client: reqwest::Client,
    remote: Option<(String, String)>,
    root: Option<PathBuf>,
}

impl Pools {
    pub(super) fn new(state: &AppState) -> Result<Self> {
        let remote = state
            .lifecycle
            .platform
            .get()
            .map(|platform| {
                let url = reqwest::Url::parse(&platform.resource_service)?;
                anyhow::ensure!(
                    url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
                    "resource service must use loopback HTTP"
                );
                let credential = state
                    .lifecycle
                    .credential
                    .get()
                    .context("resource service credential unavailable")?;
                Ok::<_, anyhow::Error>((platform.resource_service.clone(), credential.clone()))
            })
            .transpose()?;
        let config = opencoder_core::Config::load(&state.workdir)?;
        let root = config
            .dag
            .binary_dir
            .or_else(|| Some(opencoder_core::data_dir_for(&state.workdir).join("dag/binary")));
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
            remote,
            root,
        })
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let (base, token) = self.remote.as_ref().context("remote owner unavailable")?;
        let response = self
            .client
            .get(format!("{}{path}", base.trim_end_matches('/')))
            .bearer_auth(token)
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "resource owner returned {}",
            response.status()
        );
        Ok(response.json().await?)
    }

    pub(super) async fn names(&self) -> Result<Vec<String>> {
        if self.remote.is_some() {
            let value = self.get("/api/dag/binaries").await?;
            return value["pools"]
                .as_array()
                .context("invalid pool list")?
                .iter()
                .map(|p| {
                    p["name"]
                        .as_str()
                        .map(str::to_owned)
                        .context("pool name missing")
                })
                .collect();
        }
        let root = self.root.as_ref().context("binary root missing")?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        Ok(opencoder_dag_binary::list_pools(root))
    }

    pub(super) async fn detail(&self, name: &str) -> Result<Value> {
        if self.remote.is_some() {
            return self.get(&path("/api/dag/binaries/", name)).await;
        }
        let root = self.root.as_ref().context("binary root missing")?;
        let meta =
            opencoder_dag_binary::read_pool_meta(root, name).context("pool metadata unreadable")?;
        let history = meta
            .history
            .iter()
            .map(|v| {
                let meta = opencoder_dag_binary::read_version_meta(root, name, *v)
                    .context("version metadata unreadable")?;
                Ok::<_, anyhow::Error>(serde_json::to_value(meta)?)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(
            serde_json::json!({"name":meta.name,"description":meta.description,"created_at":meta.created_at,
            "updated_at":meta.updated_at,"current":meta.current,"history":history}),
        )
    }
}

pub(super) async fn with_etag(mut response: Response) -> Response {
    if !response.status().is_success() {
        return response;
    }
    let body = std::mem::replace(response.body_mut(), Body::empty());
    match axum::body::to_bytes(body, 16 * 1024 * 1024).await {
        Ok(bytes) => {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let tag = format!("\"{}\"", super::model::digest(&value));
                response
                    .headers_mut()
                    .insert("etag", HeaderValue::from_str(&tag).expect("hex digest"));
            }
            *response.body_mut() = Body::from(bytes);
            response
        }
        Err(error) => crate::api::error_500(error.to_string()),
    }
}
