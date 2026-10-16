use super::{access, error, storage, HttpError};
use crate::AppState;
use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    path: String,
}

struct Pending(PathBuf);
impl Drop for Pending {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(crate) async fn upload(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let result = async {
        storage::relative(&query.path).map_err(|_| HttpError(400, "invalid file path"))?;
        let access = access(&state, &id).await?;
        if !access.uploads_enabled {
            return Err(HttpError(503, "TODO file uploads are disabled").into());
        }
        if headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|v| v > access.max_file_bytes)
        {
            return Err(HttpError(413, "file exceeds max_file_bytes").into());
        }
        let directory = storage::path(&access.root, &access.directory)?;
        let target = storage::path(&directory, &query.path)?;
        if target.try_exists()? {
            return Err(HttpError(
                409,
                "file already exists; use another path or delete it first",
            )
            .into());
        }
        let pending =
            Pending(storage::state_dir(&access.root).join(format!("upload-{}", ulid::Ulid::new())));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending.0)
            .await?;
        let mut bytes = 0u64;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| HttpError(400, "upload interrupted"))?;
            bytes = bytes
                .checked_add(chunk.len() as u64)
                .context("file size overflow")?;
            if bytes > access.max_file_bytes {
                return Err(HttpError(413, "file exceeds max_file_bytes").into());
            }
            file.write_all(&chunk).await?;
        }
        file.sync_all().await?;
        drop(file);
        // The per-TODO lock remains held until publication, preventing deletion races.
        if state.projects.get_todo(&id).await?.is_none() {
            return Err(HttpError(404, "TODO not found").into());
        }
        storage::no_links(&target)?;
        let parent = target.parent().context("missing parent directory")?;
        std::fs::create_dir_all(parent)?;
        std::fs::hard_link(&pending.0, &target)?;
        storage::sync_parents(parent, &access.root)?;
        Ok::<_, anyhow::Error>(json!({"path":query.path,"bytes":bytes}))
    }
    .await;
    match result {
        Ok(value) => (axum::http::StatusCode::CREATED, Json(value)).into_response(),
        Err(e) => error(e),
    }
}

pub(crate) async fn download(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
) -> Response {
    let result: Result<Response> = async {
        storage::relative(&query.path).map_err(|_| HttpError(400, "invalid file path"))?;
        let access = access(&state, &id).await?;
        let directory = storage::path(&access.root, &access.directory)?;
        let target = storage::path(&directory, &query.path)?;
        let file = storage::open_file(&target)?;
        let size = file.metadata()?.len();
        let name = query.path.rsplit('/').next().unwrap_or("file");
        let encoded: String = name
            .as_bytes()
            .iter()
            .map(|b| format!("%{b:02X}"))
            .collect();
        let body = Body::from_stream(ReaderStream::new(tokio::fs::File::from_std(file)));
        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, size)
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-content-type-options", "nosniff")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"file\"; filename*=UTF-8''{encoded}"),
            )
            .body(body)?)
    }
    .await;
    result.unwrap_or_else(error)
}

pub(crate) async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
) -> Response {
    let result = async {
        storage::relative(&query.path).map_err(|_| HttpError(400, "invalid file path"))?;
        let access = access(&state, &id).await?;
        let directory = storage::path(&access.root, &access.directory)?;
        let target = storage::path(&directory, &query.path)?;
        drop(storage::open_file(&target)?);
        std::fs::remove_file(&target)?;
        opencoder_core::platform::fs::sync_directory(target.parent().context("missing parent")?)?;
        Ok::<_, anyhow::Error>(json!({"deleted":true}))
    }
    .await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => error(e),
    }
}
