//! Thin remote client for an `opencoder server`. Mirrors the server's HTTP/JSON
//! API and decodes its SSE `/events` stream back into structured events. The
//! client holds NO local data and calls NO LLM directly — every request is
//! forwarded to the server with a bearer token.

pub mod remote;
mod sse;

pub use remote::Remote;
pub use sse::SseFrame;
