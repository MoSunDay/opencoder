//! Process-level DAG functional-confirmation suite: the real
//! `opencoder-server` + `opencoder-agent` binaries, a scripted loopback LLM
//! stub, real HTTP, and compiled Linux binaries — covering D1 (spec →
//! dispatch → binary/agent steps → artifacts), D2 (versioned binary pool +
//! freeze-on-accept), D3 (cancel + step failure), the shared sandbox
//! (`runc` or the native clone3 backend, pinned through `DAG_TEST_SANDBOX`)
//! and dispatch `input.args` command-line passthrough.

#[path = "../support/mod.rs"]
mod support;

mod agent_runc;
mod binary_pool;
mod cancel_fail;
mod clone3_agent;
mod clone3_environment;
mod clone3_fixtures;
mod clone3_lifecycle;
mod clone3_probe;
mod fixtures;
mod flow;
mod input_args;
mod structured_output;

mod dynamic;
