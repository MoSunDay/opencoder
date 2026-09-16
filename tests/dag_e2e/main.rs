//! Process-level DAG functional-confirmation suite: the real
//! `opencoder-server` + `opencoder-agent` binaries, a scripted loopback LLM
//! stub, real HTTP, and wat-compiled wasm — covering the brief's D1 (spec →
//! dispatch → wasm/agent steps → artifacts), D2 (versioned wasm pool +
//! freeze-on-accept) and D3 (cancel + step failure), with the D5 runc
//! scenario skipping cleanly when the sandbox is unavailable.

#[path = "../support/mod.rs"]
mod support;

mod cancel_fail;
mod fixtures;
mod flow;
mod wasm_pool;
