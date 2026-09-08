//! Full-chain integration tests for the `opencoder-cli` surface: each test
//! boots a REAL control plane (`opencoder_control::build_app`, bearer token,
//! no web assets) on an ephemeral loopback port with its own tempdir
//! workspace — never the developer's global agents/share roots — and a
//! `MockChatClient` behind the brain seam, then drives the CLI through
//! `opencoder_cli::Cli::try_parse_from` + `opencoder_cli::run`: the exact
//! entry the `opencoder-cli` binary executes, minus the process boundary.
//!
//! Assertions use the exit-code contract (0 success / 2 auth / 4 server
//! rejection) plus server state read back over plain HTTP; the CLI's stdout
//! is deliberately never parsed (it is covered by unit tests).
//!
//! Two behaviors deviate from the plain reading of the checklist, both
//! verified against server sources and asserted as-is:
//! * `/api/ready` answers 200 only for `mode == open && ready_nodes > 0`
//!   (`api/admission.rs`); this node-less cluster answers 503 → exit 4.
//! * `drain reopen` while actually frozen needs an online node to verify
//!   (`api/admission.rs`) → 503 → exit 4; reopen in the open state
//!   short-circuits 200 → exit 0. The frozen test restores the gate directly.

mod server_local_defs;

use opencoder_control::admission::AdmissionMode;
use server_local_defs::{api_get, assert_ok, cli, Server, TOKEN};
use server_local_defs::{CAP, CAP_UPDATED, CARD, DAG_SPEC, PROMPT_PACK, TEAM, TEAM_RAW, WF_SPEC};

// ── checklist 1 + 10: probes and the bearer contract ──────────────────

#[tokio::test]
async fn system_probes_and_bearer_auth_contract() {
    let s = Server::new(None).await;
    assert_ok(&s, &["health"]).await;
    assert_ok(&s, &["time"]).await;
    // No node is linked, so `ready_nodes == 0` → 503 → exit 4 (see the
    // module doc; the frozen-mode 503 is exercised in the drain test).
    assert_eq!(cli(&s, TOKEN, &["ready"]).await, 4);
    // Wrong bearer → 401 → the dedicated auth exit code.
    assert_eq!(cli(&s, "wrong-token", &["health"]).await, 2);
    // Streaming surfaces must honor the same 2/4 classification as
    // buffered ones (regression: non-2xx SSE responses used to fall
    // through to exit 1 with a generic transport error).
    assert_eq!(
        cli(
            &s,
            "wrong-token",
            &["exec", "events", "01JDUMMYEXEC000000000000000"]
        )
        .await,
        2
    );
    // Unknown execution id on the same SSE route: control `api/stream.rs`
    // answers the lookup status immediately (verified: no node needed,
    // no hang) → 404 → exit 4.
    assert_eq!(
        cli(
            &s,
            TOKEN,
            &["exec", "events", "01JDUMMYEXEC000000000000000"]
        )
        .await,
        4
    );
    // Same contract through a relay-streaming surface (`raw --stream`).
    assert_eq!(
        cli(
            &s,
            "wrong-token",
            &["raw", "call", "GET", "/api/health", "--stream"]
        )
        .await,
        2
    );
}

// ── checklist 2: drain status/freeze/reopen over the real gate ────────

#[tokio::test]
async fn drain_cycle_against_the_real_admission_gate() {
    let s = Server::new(None).await;
    assert_ok(&s, &["drain", "status"]).await;
    assert_eq!(s.state.admission.snapshot().await.mode, AdmissionMode::Open);
    // Reopen in the open state short-circuits 200 (api/admission.rs).
    assert_ok(&s, &["drain", "reopen"]).await;
    assert_ok(&s, &["drain", "freeze"]).await;
    assert_eq!(
        s.state.admission.snapshot().await.mode,
        AdmissionMode::Frozen
    );
    // Frozen mode turns /api/ready into a 503 → exit 4.
    assert_eq!(cli(&s, TOKEN, &["ready"]).await, 4);
    assert_ok(&s, &["drain", "status"]).await;
    // Frozen + zero online nodes: reopen cannot verify on a node → 503 → 4.
    assert_eq!(cli(&s, TOKEN, &["drain", "reopen"]).await, 4);
    // Restore through the gate itself; the HTTP path needs a linked node.
    s.state.admission.reopen(&s.state.placement).await.unwrap();
    assert_eq!(s.state.admission.snapshot().await.mode, AdmissionMode::Open);
    assert_ok(&s, &["drain", "status"]).await;
}

// ── checklist 3 + 11: teams put/list and the raw escape hatch ─────────

#[tokio::test]
async fn teams_put_list_and_raw_escape_hatch() {
    let s = Server::new(None).await;
    assert_ok(&s, &["teams", "put", "--json", TEAM]).await;
    assert_ok(&s, &["teams", "list"]).await;
    let listed = api_get(&s, "/api/teams").await;
    assert_eq!(listed["teams"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["teams"][0]["captain"], "m1", "{listed}");
    // `raw` drives routes verbatim: probe GET, then the same save route the
    // dedicated `teams put` wrapper uses.
    assert_ok(&s, &["raw", "call", "GET", "/api/health"]).await;
    assert_ok(
        &s,
        &["raw", "call", "POST", "/api/teams", "--json", TEAM_RAW],
    )
    .await;
    let listed = api_get(&s, "/api/teams").await;
    let teams = listed["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 2, "{listed}");
    assert_eq!(teams[1]["name"], "t2", "{listed}");
}

// ── checklist 4: DAG definition CRUD ──────────────────────────────────

#[tokio::test]
async fn dag_definitions_crud_roundtrip() {
    let s = Server::new(None).await;
    assert_ok(&s, &["dag", "defs", "put", "--json", DAG_SPEC]).await;
    assert_ok(&s, &["dag", "defs", "list"]).await;
    assert_ok(&s, &["dag", "defs", "get", "etl-ctl"]).await;
    let def = api_get(&s, "/api/dag/defs/etl-ctl").await;
    assert_eq!(def["spec"]["name"], "etl-ctl", "{def}");
    assert_ok(&s, &["dag", "defs", "delete", "etl-ctl"]).await;
    // Deleted → 404 → the server-rejection exit code.
    assert_eq!(cli(&s, TOKEN, &["dag", "defs", "get", "etl-ctl"]).await, 4);
}

// ── checklist 5: todo envs + templates on an isolated share tree ──────

/// The share-tree APIs resolve their root through a process-global override
/// BEFORE any config (`share_fs::effective_share_dir`), so the todo tests
/// serialize on a local gate — the control e2e SHARE_GATE pattern — and pin
/// the override (and the workspace config, belt and braces) at a tempdir.
static SHARE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn todo_envs_and_templates_full_lifecycle() {
    let _guard = SHARE_GATE.lock().await;
    let share = tempfile::tempdir().unwrap();
    let share_root = share.path().to_path_buf();
    opencoder_core::set_share_dir_override(Some(share_root.clone()));
    let s = Server::new(Some(share_root)).await;

    assert_ok(
        &s,
        &[
            "todo",
            "envs",
            "put",
            "--json",
            r#"{"name":"dev","description":"ctl env"}"#,
        ],
    )
    .await;
    assert_ok(&s, &["todo", "envs", "get", "dev"]).await;
    assert_ok(&s, &["todo", "envs", "list"]).await;
    assert_ok(
        &s,
        &[
            "todo",
            "envs",
            "update",
            "dev",
            "--json",
            r#"{"description":"patched"}"#,
        ],
    )
    .await;

    let template = format!(r#"{{"name":"demo","spec":{WF_SPEC}}}"#);
    assert_ok(&s, &["todo", "templates", "put", "--json", &template]).await;
    assert_ok(&s, &["todo", "templates", "get", "demo"]).await;
    assert_ok(&s, &["todo", "templates", "list"]).await;
    assert_ok(&s, &["todo", "templates", "get-meta", "demo"]).await;
    assert_ok(
        &s,
        &[
            "todo",
            "templates",
            "new-version",
            "demo",
            "--json",
            r#"{"source_version":"v1"}"#,
        ],
    )
    .await;
    assert_ok(&s, &["todo", "templates", "delete", "demo"]).await;
    assert_ok(&s, &["todo", "envs", "delete", "dev"]).await;

    // Leave no stale override pointing at a dropped tempdir behind.
    opencoder_core::set_share_dir_override(None);
}

// ── checklist 6: project goals CRUD ───────────────────────────────────

#[tokio::test]
async fn project_goals_create_list_patch_delete() {
    let s = Server::new(None).await;
    assert_ok(
        &s,
        &[
            "project",
            "goals",
            "create",
            "--json",
            r#"{"title":"G1","detail_md":"d"}"#,
        ],
    )
    .await;
    let goals = api_get(&s, "/api/project/goals").await;
    let goal_id = goals["goals"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(goals["goals"][0]["title"], "G1", "{goals}");
    assert_ok(&s, &["project", "goals", "list"]).await;
    assert_ok(
        &s,
        &[
            "project",
            "goals",
            "patch",
            &goal_id,
            "--json",
            r#"{"status":"archived"}"#,
        ],
    )
    .await;
    assert_ok(&s, &["project", "goals", "delete", &goal_id]).await;
    let goals = api_get(&s, "/api/project/goals").await;
    assert_eq!(goals["goals"].as_array().unwrap().len(), 0, "{goals}");
}

// ── checklist 7: brain capabilities + search ──────────────────────────

#[tokio::test]
async fn brain_caps_lifecycle_and_search() {
    let s = Server::new(None).await;
    assert_ok(&s, &["brain", "caps", "create", "--json", CAP]).await; // 201
    let caps = api_get(&s, "/api/brain/capabilities").await;
    let cap_id = caps["capabilities"][0]["capability"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ok(&s, &["brain", "caps", "get", &cap_id]).await;
    assert_ok(&s, &["brain", "caps", "list"]).await;
    assert_ok(
        &s,
        &["brain", "caps", "update", &cap_id, "--json", CAP_UPDATED],
    )
    .await;
    // Semantic search over the mock-embedded library; blank query → 400 → 4.
    assert_ok(
        &s,
        &[
            "brain",
            "search",
            "--json",
            r#"{"query":"route database work"}"#,
        ],
    )
    .await;
    assert_eq!(
        cli(
            &s,
            TOKEN,
            &["brain", "search", "--json", r#"{"query":"  "}"#]
        )
        .await,
        4
    );
    assert_ok(&s, &["brain", "caps", "delete", &cap_id]).await;
}

// ── checklist 8: custom agents card lifecycle ─────────────────────────

#[tokio::test]
async fn agents_card_lifecycle_and_active_pointer() {
    let s = Server::new(None).await;
    // Activation requires the referenced prompts pool to exist first.
    assert_ok(
        &s,
        &[
            "agents",
            "resources",
            "create",
            "prompts",
            "--json",
            PROMPT_PACK,
        ],
    )
    .await;
    assert_ok(&s, &["agents", "create", "--json", CARD]).await; // 201
    assert_ok(&s, &["agents", "active", "--json", r#"{"active":"alpha"}"#]).await;
    assert_ok(&s, &["agents", "meta", "alpha"]).await;
    assert_ok(&s, &["agents", "list"]).await;
    let listed = api_get(&s, "/api/agents").await;
    assert_eq!(listed["active"], "alpha", "{listed}");
    assert_eq!(listed["agents"].as_array().unwrap().len(), 1, "{listed}");
    assert_ok(&s, &["agents", "active", "--json", r#"{"active":null}"#]).await;
    assert_ok(&s, &["agents", "delete", "alpha"]).await;
}

// ── checklist 9 + 12: executions list and the 4xx contract ────────────

#[tokio::test]
async fn exec_list_empty_and_missing_get_rejected() {
    let s = Server::new(None).await;
    assert_ok(&s, &["exec", "list"]).await;
    let list = api_get(&s, "/api/executions").await;
    assert_eq!(list["executions"].as_array().unwrap().len(), 0, "{list}");
    // Unknown execution id → 404 → the server-rejection exit code.
    assert_eq!(cli(&s, TOKEN, &["exec", "get", "exec-none"]).await, 4);
}
