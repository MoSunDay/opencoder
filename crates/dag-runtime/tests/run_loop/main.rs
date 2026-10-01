//! Integration tests for the whole-run scheduling loop (`execute_run`):
//! a local axum stub stands in for the server's DAG uplink endpoints and
//! captures every event batch + terminal status report, while the agent
//! step runs on the real session runner with a `MockChatClient`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use opencoder_dag::protocol::{DagClaimedRun, DagEventBatch, DagStatusReport};
use opencoder_dag::{DagEventIn, DagRunStatus, DagSpec, StepKind, StepSpec};
use opencoder_dag_runtime::{execute_run, ExecDeps, RunDeps};
use opencoder_llm::event::LlmEvent;
use opencoder_llm::MockChatClient;
use opencoder_node::uplink::Uplink;
use opencoder_store::LibsqlStore;
use serde_json::json;
use tokio::net::TcpListener;

#[path = "../support/container.rs"]
mod container;
#[path = "../support/model.rs"]
mod model;

mod helpers;
use helpers::*;
mod concurrency;
mod coordination;
mod flow;
mod native_session;
mod policies;
