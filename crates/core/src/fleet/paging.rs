use super::protocol::ExecutionRef;
use serde::{Deserialize, Serialize};

pub const EXECUTION_PAGE_DEFAULT: u32 = 100;
pub const EXECUTION_PAGE_MAX: u32 = 500;
pub const MESSAGE_CHUNK_BYTES: usize = 64 * 1024;
pub const MESSAGE_PAGE_RAW_BYTES: usize = 11 * MESSAGE_CHUNK_BYTES;
pub const QUERY_RESPONSE_BYTES: usize = 1024 * 1024;
pub const EVENT_PAGE_MAX: u32 = 200;
pub const ARTIFACT_CHUNK_BYTES: usize = 64 * 1024;
pub const EVENT_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionCursor {
    pub created_at: i64,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionPage<T> {
    pub executions: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<ExecutionCursor>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageCursor {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageChunk {
    pub seq: i64,
    pub role: String,
    pub created_at: i64,
    pub offset: u64,
    pub next_offset: u64,
    pub total_bytes: u64,
    pub eof: bool,
    pub encoding: String,
    pub bytes_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePage {
    pub chunks: Vec<MessageChunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<MessageCursor>,
    pub more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventPayloadRequest {
    pub execution: ExecutionRef,
    pub seq: i64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventPayloadChunk {
    pub seq: i64,
    pub offset: u64,
    pub next_offset: u64,
    pub total_bytes: u64,
    pub eof: bool,
    pub encoding: String,
    pub bytes_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetailFieldRequest {
    pub execution: ExecutionRef,
    pub field: String,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetailFieldChunk {
    pub field: String,
    pub offset: u64,
    pub next_offset: u64,
    pub total_bytes: u64,
    pub eof: bool,
    pub encoding: String,
    pub bytes_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRequest {
    pub execution: ExecutionRef,
    pub step: String,
    pub file: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChunk {
    pub step: String,
    pub file: String,
    pub offset: u64,
    pub next_offset: u64,
    pub total_bytes: u64,
    pub version: String,
    pub eof: bool,
    pub encoding: String,
    pub bytes_b64: String,
}
