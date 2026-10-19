use super::{
    model, references,
    transport::{self, Pools},
};
use crate::AppState;
use anyhow::{Context, Result};
use axum::{body::Body, extract::Request};
use base64::Engine;
use futures::{stream, StreamExt, TryStreamExt};
use opencoder_core::fleet::RpcReply;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) async fn check(state: &AppState, request: &mut Request) -> Result<Option<RpcReply>> {
    let path = request.uri().path().to_owned();
    let name = path.strip_prefix("/api/dag/binaries/");
    if path != "/api/dag/binaries" && name.is_none_or(|n| n.contains('/') || n == "nfs") {
        return Ok(None);
    }
    let method = request.method().as_str().to_owned();
    if !matches!(method.as_str(), "POST" | "PUT" | "DELETE") {
        return Ok(None);
    }
    if (method == "POST") != name.is_none() {
        return Ok(None);
    }
    let pools = std::sync::Arc::new(Pools::new(state)?);
    let names = pools.names().await?;
    if method == "DELETE" {
        let Some(name) = name else { return Ok(None) };
        if !names.iter().any(|n| n == name) {
            return Ok(Some(RpcReply::error(404, "unknown binary pool")));
        }
        let pool = pools.detail(name).await?;
        if let Err(reply) = model::unchanged(request.headers(), &pool) {
            return Ok(Some(reply));
        }
        let found = references::find(state, name, false).await?;
        if !found.is_empty() {
            return Ok(Some(model::conflict(
                "catalog_in_use",
                format!("资源仍被引用: {}", found.join(", ")),
                json!({"references":found}),
            )));
        }
        super::retired::mark(state, "binary", name).await?;
        return Ok(None);
    }
    let bytes = axum::body::to_bytes(
        std::mem::replace(request.body_mut(), Body::empty()),
        48 * 1024 * 1024,
    )
    .await?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    *request.body_mut() = Body::from(bytes);
    let body = match parsed {
        Ok(body) => body,
        Err(_) => return Ok(None),
    };
    let name = name.or_else(|| body["name"].as_str()).unwrap_or("").trim();
    if opencoder_dag_binary::validate_name(name).is_err() {
        return Ok(Some(RpcReply::error(400, "invalid binary pool name")));
    }
    let Some(encoded) = body["binary_b64"].as_str() else {
        return Ok(None);
    };
    let binary = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(binary) => binary,
        Err(_) => return Ok(Some(RpcReply::error(400, "invalid base64 binary"))),
    };
    if binary.len() > 32 * 1024 * 1024 {
        return Ok(Some(RpcReply::error(400, "binary exceeds 32 MiB")));
    }
    if let Err(error) = opencoder_dag_binary::validate_binary_bytes(&binary) {
        return Ok(Some(RpcReply::error(400, error.to_string())));
    }
    let hash = format!("{:x}", Sha256::digest(&binary));
    let size = binary.len() as u64;
    let present = names.iter().any(|n| n == name);
    if method == "PUT" && !present {
        return Ok(Some(RpcReply::error(404, "unknown binary pool")));
    }
    let mut details: Vec<Value> = stream::iter(names)
        .map(|n| {
            let pools = pools.clone();
            async move { pools.detail(&n).await }
        })
        .buffered(8)
        .try_collect()
        .await?;
    details.sort_by_key(|p| {
        (
            p["name"] != name,
            p["name"].as_str().unwrap_or("").to_owned(),
        )
    });
    for pool in &details {
        for version in pool["history"]
            .as_array()
            .context("invalid binary history")?
        {
            anyhow::ensure!(!version.is_null(), "binary version metadata unreadable");
            if version["sha256"] != hash || version["size_bytes"].as_u64() != Some(size) {
                continue;
            }
            if pool["name"] == name
                && pool["current"] == version["version"]
                && pool["description"].as_str().unwrap_or("")
                    == body["description"].as_str().unwrap_or("")
            {
                return Ok(Some(RpcReply::ok(
                    json!({"ok":true,"name":name,"version":version["version"],"unchanged":true}),
                )));
            }
            let existing = pool["name"].as_str().context("pool name missing")?;
            return Ok(Some(model::conflict(
                "duplicate_resource",
                format!(
                    "已有相同二进制，请复用 {existing}@v{}；历史版本请显式切换",
                    version["version"]
                ),
                json!({"name":existing,"version":version["version"],"url":transport::path("/api/dag/binaries/",existing)}),
            )));
        }
    }
    Ok(None)
}
