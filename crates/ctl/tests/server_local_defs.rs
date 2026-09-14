//! Support module for `server_local.rs`: the in-process control-plane
//! harness (tempdir workspace + ephemeral port + `MockChatClient`) and the
//! legal request bodies copied from the control-plane e2e suites
//! (`teams_dag_defs.rs`, `brain_api/capabilities.rs`, `agents_api.rs`,
//! todo template/env specs). Pure data + functions; no tests live here.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use opencoder_llm::{ChatStream, MockChatClient};
use serde_json::Value;

pub const TOKEN: &str = "ctl-server-local-token";

/// Legal TeamDefinition (shape copied from control e2e `teams_dag_defs.rs`).
pub const TEAM: &str = r#"{"name":"t1","captain":"act","members":[
    {"agent":"act"},
    {"agent":"plan"}]}"#;

/// Second team, used to prove the `raw` escape hatch is equivalent to
/// `teams put` (upsert keyed by name, distinct name → second row).
pub const TEAM_RAW: &str = r#"{"name":"t2","captain":"act","members":[{"agent":"act"}]}"#;

/// Legal DagSpec (bare spec, no `{"spec":…}` wrapper) from the same file.
pub const DAG_SPEC: &str = r#"{"name":"etl-ctl","steps":[
    {"name":"fetch","kind":{"type":"wasm","command":"tool.wasm"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"wasm","command":"tool.wasm"}}]}"#;

/// Minimal valid single-TODO WorkflowSpec (control e2e `spec()` helper).
pub const WF_SPEC: &str = r#"{"schema_version":1,"id":"wf-ctl-demo","name":"demo",
    "objective":"ship it","todos":[{"id":"t1","title":"T1","requirement_background":"bg",
    "instructions":"do it","agent":"act","acceptance":{"criteria":"c"}}],"metadata":{}}"#;

/// Full CapabilityInput (control e2e `brain_api/capabilities.rs`).
pub const CAP: &str = r#"{"capability_type":"tool-usage","summary":"ctl capability summary",
    "input_desc":"a work request","output_desc":"completed work",
    "eng_inputs":["exemplar input"]}"#;

pub const CAP_UPDATED: &str = r#"{"capability_type":"tool-usage","summary":"updated summary",
    "input_desc":"a work request","output_desc":"completed work",
    "eng_inputs":["exemplar input"]}"#;

/// Prompts pool the alpha card activates against (activation preflight
/// resolves `current.prompt` to a live pool version — see web api_agents.rs).
/// `content_b64` is base64("ctl pack body").
pub const PROMPT_PACK: &str = r#"{"name":"pack","files":[
    {"path":"soul.md","content_b64":"Y3RsIHBhY2sgYm9keQ=="}]}"#;

pub const CARD: &str = r#"{"name":"alpha","current":{"prompt":"pack"}}"#;

/// One real control plane per test: own tempdir workspace (never the
/// developer's global agents/share roots) and an ephemeral loopback port.
/// The TempDir must outlive every CLI call, so it lives in the struct and
/// drops last.
pub struct Server {
    pub base: String,
    pub state: Arc<opencoder_control::AppState>,
    task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Server {
    /// Boots the server. `share_dir` also pins `agent.share_dir` in the
    /// workspace config for the share-tree (todo) APIs.
    pub async fn new(share_dir: Option<PathBuf>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut agent = serde_json::json!({"agents_dir": dir.path().join("agents")});
        if let Some(share) = share_dir {
            agent["share_dir"] = serde_json::json!(share);
        }
        std::fs::write(
            work.join("opencoder.json"),
            serde_json::json!({"agent": agent}).to_string(),
        )
        .unwrap();
        let mock = Arc::new(MockChatClient::new());
        let state = opencoder_control::new_state(
            work,
            dir.path().join("data"),
            Some(mock as Arc<dyn ChatStream>),
        )
        .await
        .unwrap();
        let app = opencoder_control::build_app(state.clone(), Some(TOKEN.into()), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base,
            state,
            task,
            _dir: dir,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Run `opencoder-cli --server <base> --token <token> <args…>` in-process
/// and return the exit code `run` settled on.
pub async fn cli(server: &Server, token: &str, args: &[&str]) -> i32 {
    let mut argv: Vec<String> = vec![
        "opencoder-cli".into(),
        "--server".into(),
        server.base.clone(),
        "--token".into(),
        token.into(),
    ];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    let parsed = opencoder_cli::Cli::try_parse_from(argv).unwrap();
    opencoder_cli::run(parsed).await.unwrap()
}

/// Assert the success path (exit 0) for a full argv tail.
pub async fn assert_ok(server: &Server, args: &[&str]) {
    assert_eq!(
        cli(server, TOKEN, args).await,
        0,
        "expected exit 0 for {args:?}"
    );
}

/// Read one JSON route straight off the server (assertion channel; the
/// CLI's own stdout is deliberately never parsed here).
pub async fn api_get(server: &Server, path: &str) -> Value {
    let client = opencoder_cli::http::client().unwrap();
    client
        .get(format!("{}{}", server.base, path))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}
