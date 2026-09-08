use crate::{api::response, AppState};
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine;
use opencoder_core::fleet::*;
use serde::Deserialize;
use std::{io, sync::Arc};

#[derive(Debug, Deserialize)]
pub struct ArtifactQuery {
    step: String,
    file: Option<String>,
}

pub async fn download(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ArtifactQuery>,
) -> Response {
    let file = query.file.unwrap_or_else(|| "output.txt".into());
    let index = match state.fleet.index(&id).await {
        Ok(Some(index)) if matches!(index.kind, ExecutionKind::Dag | ExecutionKind::Project) => {
            index
        }
        Ok(Some(_)) => return response(RpcReply::error(400, "artifacts require a DAG execution")),
        Ok(None) => return response(RpcReply::error(404, "execution id not found")),
        Err(error) => return response(RpcReply::error(500, format!("index: {error:#}"))),
    };
    let first = fetch(&state, &index, &query.step, &file, 0, None).await;
    let first = match decode_reply(first, 0, None, None) {
        Ok(chunk) => chunk,
        Err(reply) => return response(reply),
    };
    let total = first.0.total_bytes;
    let version = first.0.version.clone();
    let filename = safe_filename(&format!("{}-{}-{}", id, query.step, file));
    let stream = futures::stream::unfold(
        StreamState {
            state,
            index,
            step: query.step,
            file,
            total,
            version,
            next: 0,
            pending: Some(first),
            done: false,
        },
        next_chunk,
    );
    let mut response = Body::from_stream(stream).into_response();
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&total.to_string()).expect("u64 is a valid header"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .expect("sanitized filename is a valid header"),
    );
    response
}

struct StreamState {
    state: Arc<AppState>,
    index: ExecutionIndex,
    step: String,
    file: String,
    total: u64,
    version: String,
    next: u64,
    pending: Option<(ArtifactChunk, Vec<u8>)>,
    done: bool,
}

async fn next_chunk(mut state: StreamState) -> Option<(Result<Bytes, io::Error>, StreamState)> {
    if state.done {
        return None;
    }
    let (chunk, bytes) = match state.pending.take() {
        Some(chunk) => chunk,
        None => {
            let reply = fetch(
                &state.state,
                &state.index,
                &state.step,
                &state.file,
                state.next,
                Some(&state.version),
            )
            .await;
            match decode_reply(reply, state.next, Some(state.total), Some(&state.version)) {
                Ok(chunk) => chunk,
                Err(error) => return Some((Err(stream_error(error)), state)),
            }
        }
    };
    state.next = chunk.next_offset;
    state.done = chunk.eof;
    Some((Ok(Bytes::from(bytes)), state))
}

async fn fetch(
    state: &AppState,
    index: &ExecutionIndex,
    step: &str,
    file: &str,
    offset: u64,
    version: Option<&str>,
) -> RpcReply {
    state
        .hub
        .call(
            &index.node_id,
            NodeOperation::Artifact {
                request: ArtifactRequest {
                    execution: index.execution_ref(),
                    step: step.to_owned(),
                    file: file.to_owned(),
                    offset,
                    version: version.map(str::to_owned),
                },
            },
        )
        .await
}

fn decode_reply(
    reply: RpcReply,
    offset: u64,
    total: Option<u64>,
    version: Option<&str>,
) -> Result<(ArtifactChunk, Vec<u8>), RpcReply> {
    if reply.status != 200 {
        return Err(reply);
    }
    let chunk: ArtifactChunk = serde_json::from_value(reply.body)
        .map_err(|_| RpcReply::error(502, "node returned an invalid artifact chunk"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&chunk.bytes_b64)
        .map_err(|_| RpcReply::error(502, "node returned invalid artifact base64"))?;
    if chunk.encoding != "base64"
        || chunk.offset != offset
        || total.is_some_and(|value| value != chunk.total_bytes)
        || version.is_some_and(|value| value != chunk.version)
        || chunk.version.is_empty()
        || chunk.next_offset != offset + bytes.len() as u64
        || chunk.next_offset > chunk.total_bytes
        || chunk.eof != (chunk.next_offset >= chunk.total_bytes)
        || bytes.len() > ARTIFACT_CHUNK_BYTES
        || (bytes.is_empty() && !chunk.eof)
    {
        return Err(RpcReply::error(
            502,
            "node returned inconsistent artifact metadata",
        ));
    }
    Ok((chunk, bytes))
}

fn stream_error(reply: RpcReply) -> io::Error {
    io::Error::other(format!(
        "artifact node stream failed ({}): {}",
        reply.status, reply.body
    ))
}

fn safe_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .take(180)
        .collect()
}
