//! Process-level e2e for the team face: real server + agent binaries, a real
//! node registering over WebSocket, and a scripted LLM stub driving the team
//! runtime's plan/answer/summary/closing decisions to a completed topic.
#![allow(clippy::too_many_arguments)]

#[path = "../support/mod.rs"]
mod support;

mod flow;
