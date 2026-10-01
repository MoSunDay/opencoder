//! Bounded full messages for remote TUI restore through execution commands.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use opencoder_core::fleet::{
    MessageChunk, MessageCursor, MessagePage, MESSAGE_CHUNK_BYTES, MESSAGE_PAGE_RAW_BYTES,
};
use std::sync::Arc;

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(cursor): Query<MessageCursor>,
) -> Response {
    let result = async {
        anyhow::ensure!(
            state.store.get_session(&id).await?.is_some(),
            "session not found"
        );
        let page = state
            .store
            .load_transcript_page(&id, cursor, MESSAGE_CHUNK_BYTES, MESSAGE_PAGE_RAW_BYTES)
            .await?;
        Ok::<_, anyhow::Error>(MessagePage {
            chunks: page
                .chunks
                .into_iter()
                .map(|chunk| {
                    let next_offset = chunk.offset + chunk.bytes.len() as u64;
                    MessageChunk {
                        seq: chunk.seq,
                        role: chunk.role,
                        created_at: chunk.created_at,
                        offset: chunk.offset,
                        next_offset,
                        total_bytes: chunk.total_bytes,
                        eof: next_offset == chunk.total_bytes,
                        encoding: "base64".into(),
                        bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
                    }
                })
                .collect(),
            more: page.next_cursor.is_some(),
            next_cursor: page.next_cursor,
        })
    }
    .await;
    match result {
        Ok(page) => Json(page).into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":format!("transcript: {error:#}")})),
        )
            .into_response(),
    }
}
