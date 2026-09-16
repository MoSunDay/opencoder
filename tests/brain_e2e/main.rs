//! Process-level brain-face functional-confirmation suite: the real
//! `opencoder-server` + `opencoder-agent` binaries, a scripted loopback LLM
//! stub, and real HTTP — covering B1 (pin immutable plan version → fixed run
//! completes through a real child agent session and a receipt-bound route)
//! and B2 (bypass guard, foreign-receipt blocked fold, cancel-to-terminal).

#[path = "../support/mod.rs"]
mod support;

mod fixtures;
mod flow;
mod lifecycle;
