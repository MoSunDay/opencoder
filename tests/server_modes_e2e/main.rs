//! Process-level e2e for the `opencoder-server` binary's startup and
//! deployment surface (no agent, no LLM except the SIGTERM scenario):
//! - `cli_reject` — every fail-fast rejection path (mutually exclusive
//!   flags, missing/empty credential files, missing token) exits non-zero
//!   with a stable stderr message BEFORE any port is bound.
//! - `token_modes` — the platform server's auth surface: `--token-file`
//!   (newline tolerated), env-only token, `--auth-enabled false`, the
//!   metrics token being scoped to `GET /metrics` only, and the `--web`
//!   flag's real CLI contract.
//! - `serve_modes` — the standalone `--resources` / `--todo-resources`
//!   loopback services (no readiness line, hard-coded bind) and the
//!   one-time `schedules.json` seed import across restarts.
//! - `shutdown` — SIGTERM graceful retirement: an open SSE stream gets a
//!   final `event: reconnect` frame, then the process exits cleanly.

#[path = "../support/mod.rs"]
mod support;

mod cli_reject;
mod proc;
mod serve_modes;
mod shutdown;
mod token_modes;
