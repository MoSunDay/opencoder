//! Process-level TODO-workflow functional-confirmation suite: the real
//! `opencoder-server` + `opencoder-agent` binaries, a scripted loopback LLM
//! stub, and real HTTP — covering the brief's T1 (template → run → completed
//! with a passed item), T2 (interrupt → node respawn → resume → done) and
//! T3 (child LLM failure → todo failed + runtime suspension fold).

#[path = "../support/mod.rs"]
mod support;

mod fixtures;
mod flow;
mod lifecycle;
