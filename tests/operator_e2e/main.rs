//! Operator (process-level) e2e suite: real `opencoder-server` + real
//! `opencoder-agent` processes, a scripted loopback LLM stub and raw
//! HTTP/SSE. Scenarios:
//! - O1 `flow` — create an operator session, the agent executes the
//!   prompt on the node, the execution folds to `idle` with the
//!   session-shaped result, messages and SSE agree.
//! - O2 `relay_sse` — the `/api/sessions/:id/*` relay fallback: follow-up
//!   prompts, reads, the operation-rejection contract and the live SSE.
//! - O3 `gating` — the role gate: non-admin tokens submit operator
//!   executions (positive) and are refused everywhere else.
//! - O4 `lifecycle` — interrupt mid-drain and agent-restart recovery.

#[path = "../support/mod.rs"]
mod support;

mod flow;
mod gating;
mod lifecycle;
mod relay_sse;
