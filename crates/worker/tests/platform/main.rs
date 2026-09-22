#[path = "../support/mod.rs"]
mod support;
use opencoder_llm::LlmEvent;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

mod dag_diamond_flow;
mod dag_run_steps;
mod dag_team_loop;
mod team_multiround_consensus;
mod workloads;
