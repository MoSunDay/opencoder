//! Operator (process-level) e2e suite: real `opencoder-server` + real
//! `opencoder-agent` processes, a scripted loopback LLM stub and raw
//! HTTP/SSE. Scenarios:
//! - O1 `flow` — create an operator session, the agent executes the
//!   prompt on the node, the execution folds to `idle` with the
//!   session-shaped result, messages and SSE agree.
//! - O2 `relay_sse` — the `/api/sessions/:id/*` relay fallback: follow-up
//!   prompts, reads, the operation-rejection contract and the live SSE.
//! - O3 `gating` — the role gate: non-admin tokens submit operator and
//!   agent executions (positive) and are refused everywhere else.
//! - O4 `lifecycle` — interrupt mid-drain and agent-restart recovery.
//! - O5 `agent_session` — `kind=agent` sessions: how_append injection,
//!   output contract and the merged chat listing.
//! - O6 `agent_sandbox` — agent-card `run_mode` dispatch: fail-closed
//!   admission without a sandbox runtime, the full runc round contract
//!   (bundle shape, runner artifacts, follow-up turns) and the
//!   operator-mode host loop.
//! - O7 `isolation` — per-execution HOME/WORKSPACE: the frozen config
//!   snapshot (0600, key preserved), bash cwd/HOME verified from inside a
//!   real turn, and the pair rebuilt after an agent restart.

#[path = "../support/mod.rs"]
mod support;

mod agent_sandbox;
mod agent_session;
mod flow;
mod gating;
mod isolation;
mod lifecycle;
mod relay_sse;
