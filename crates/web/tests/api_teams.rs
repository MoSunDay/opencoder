//! HTTP contract for the team feature (`/api/teams*`, `/api/topics`):
//! signature coverage, team create/list/patch/members validation, a full
//! scripted topic run (plan → answer → summary → closing) verified against
//! the on-disk share layout, both cancel paths (hub-hit + disk), resume
//! semantics, cross-team listing with the `?team=` filter, and background
//! profiling. Mirrors the harness style of `nodes_api.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_store::{
    LibsqlStore, NodeRecord, Store, TeamTopicRunRecord, TEAM_RUN_EXECUTING, TEAM_RUN_FINISHED,
};
use opencoder_team::fs_store;
use opencoder_team::types::{MemberRef, TeamMember, TeamMeta, TopicMeta};
use opencoder_team::{ok, MockDispatcher, TeamDispatcher, TeamRunConfig};
use opencoder_web::team_state::TeamWebState;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

mod support;

const TOKEN: &str = "teams-test-token";

// ── harness ───────────────────────────────────────────────────────────────

/// One isolated deployment: throwaway store + throwaway team share. The app
/// is rebuilt per scenario over the SAME env, so a test can register nodes
/// first and then script a dispatcher that already knows their ids.
struct Env {
    store: Arc<dyn Store>,
    team_root: PathBuf,
    _root: TempDir,
    _db: TempDir,
}

async fn env() -> Env {
    let db = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open(db.path().join("t.db")).await.unwrap());
    Env {
        store,
        team_root: root.path().to_path_buf(),
        _root: root,
        _db: db,
    }
}

async fn register(env: &Env, name: &str) -> NodeRecord {
    env.store
        .register_node(name, Some("v1"), Some("/tmp/wd"), None, 1_000)
        .await
        .unwrap()
}

fn app_for(env: &Env, dispatcher: Arc<dyn TeamDispatcher>, token: Option<&str>) -> axum::Router {
    let state = Arc::new(opencoder_web::AppState {
        brain: opencoder_web::api_brain::mock_brain(env.store.clone()),
        store: env.store.clone(),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        project: opencoder_web::ProjectService::new(),
        team: Arc::new(TeamWebState::new(
            TeamRunConfig {
                team_root: env.team_root.clone(),
                max_turns: 8,
                max_sub_turns: 3,
            },
            dispatcher,
        )),
        client_override: None,
    });
    opencoder_web::build_app(state, token.map(str::to_string), false)
}

fn plain_app(env: &Env, dispatcher: Arc<dyn TeamDispatcher>) -> axum::Router {
    app_for(env, dispatcher, None)
}

fn req(method: &str, uri: &str, token: Option<&str>, body: Option<String>) -> Request<Body> {
    if let Some(t) = token {
        return support::signed_req(method, uri, t, body);
    }
    let b = Request::builder().method(method).uri(uri);
    match body {
        Some(json) => b
            .header("content-type", "application/json")
            .body(Body::from(json)),
        None => b.body(Body::empty()),
    }
    .unwrap()
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.expect("router must answer");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };
    (status, body)
}

/// Compact one-shot JSON call.
async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send(app, req(method, uri, None, body.map(|b| b.to_string()))).await
}

/// Poll `GET uri` until `pred` holds, bounded by `budget` (runtimes run as
/// spawned tasks; the HTTP call only triggers them).
async fn poll_until<F>(app: &axum::Router, uri: &str, budget: Duration, pred: F) -> Value
where
    F: Fn(&Value) -> bool,
{
    let start = Instant::now();
    loop {
        let (status, body) = call(app, "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if pred(&body) {
            return body;
        }
        assert!(
            start.elapsed() < budget,
            "timed out waiting for {uri}; last: {body}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn make_team(app: &axum::Router, name: &str, captain: &str, members: &[String]) {
    let body = json!({"name": name, "captain_node_id": captain, "member_node_ids": members});
    let (status, body) = call(app, "POST", "/api/teams", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

// ── decision-JSON builders (same shapes as the team crate's own tests) ────

fn plan(question: &str, participants: &[String]) -> String {
    json!({"question": question, "participants": participants, "rationale": "理由"}).to_string()
}

fn summary(text: &str, aligned: bool) -> String {
    json!({"summary": text, "aligned": aligned, "ambiguities": []}).to_string()
}

fn closing_complete(final_summary: &str) -> String {
    json!({"complete": true, "next_question": null, "final_summary": final_summary}).to_string()
}

/// Happy-path script: 1 round, aligned, complete.
fn happy_script(captain: &NodeRecord, member: &NodeRecord) -> MockDispatcher {
    MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok(plan("目录怎么组织", std::slice::from_ref(&member.id))),
                ok(summary("全员一致", true)),
                ok(closing_complete("最终结论：按功能分目录")),
            ],
        )
        .reply(&member.id, vec![ok("成员0：按功能分目录")])
}

/// Hand-write a topic file straight onto the share (a topic left behind by a
/// previous server process).
fn hand_topic(root: &Path, team: &str, status: &str, reason: Option<&str>, created: i64) -> String {
    let meta = TopicMeta {
        topic_id: ulid::Ulid::new().to_string(),
        team_name: team.to_string(),
        title: "手造话题".into(),
        requirement: "测试".into(),
        status: status.to_string(),
        finish_reason: reason.map(str::to_string),
        created_at: created,
        finished_at: None,
        captain: MemberRef {
            node_id: "0CAPTAIN0000000000000000".into(),
            name: "captain".into(),
        },
        members: vec![],
        turns: vec![],
        final_summary: None,
    };
    let topic_id = meta.topic_id.clone();
    std::fs::create_dir_all(root.join(team).join(&topic_id)).unwrap();
    fs_store::save_topic(root, &meta).unwrap();
    topic_id
}

fn hand_team(root: &Path, captain: &NodeRecord, member: &NodeRecord, name: &str) {
    fs_store::create_team(
        root,
        &TeamMeta {
            name: name.into(),
            captain: MemberRef {
                node_id: captain.id.clone(),
                name: captain.name.clone(),
            },
            members: vec![TeamMember {
                node_id: member.id.clone(),
                name: member.name.clone(),
                capabilities: vec![],
                profiled_at: None,
            }],
            created_at: 1,
            updated_at: 1,
        },
    )
    .unwrap();
}

// ── auth ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn team_routes_sit_behind_the_signature_middleware() {
    let env = env().await;
    let app = app_for(&env, Arc::new(MockDispatcher::new()), Some(TOKEN));
    let (status, _) = send(&app, req("GET", "/api/teams", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "unsigned request");
    let (status, body) = send(&app, req("GET", "/api/teams", Some(TOKEN), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// ── teams ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_team_list_conflict_and_unregistered_nodes() {
    let env = env().await;
    let app = plain_app(&env, Arc::new(MockDispatcher::new()));
    let captain = register(&env, "captain-a").await;
    let member = register(&env, "member-a").await;

    // Happy path: 201 with captain + member rows echoing the node registry;
    // duplicate entries in member_node_ids dedup.
    let (status, body) = call(
        &app,
        "POST",
        "/api/teams",
        Some(json!({
            "name": "alpha",
            "captain_node_id": captain.id,
            "member_node_ids": [member.id.clone(), member.id.clone()]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["team"]["name"], "alpha");
    assert_eq!(body["team"]["captain"]["node_id"], captain.id.as_str());
    assert_eq!(body["team"]["captain"]["name"], "captain-a");
    assert_eq!(
        body["team"]["members"].as_array().unwrap().len(),
        1,
        "{body}"
    );
    assert_eq!(body["team"]["members"][0]["node_id"], member.id.as_str());

    // GET list contains it.
    let (status, list) = call(&app, "GET", "/api/teams", None).await;
    assert_eq!(status, StatusCode::OK);
    let teams = list["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0]["name"], "alpha");

    // Duplicate name → 409; invalid name → 400.
    let body = json!({"name": "alpha", "captain_node_id": captain.id});
    let (status, _) = call(&app, "POST", "/api/teams", Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let body = json!({"name": "Bad Name", "captain_node_id": captain.id});
    let (status, body) = call(&app, "POST", "/api/teams", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Unregistered member node → 400 naming the node id; captain too.
    let body =
        json!({"name": "beta", "captain_node_id": captain.id, "member_node_ids": ["01GHOST"]});
    let (status, body) = call(&app, "POST", "/api/teams", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("01GHOST"),
        "error must name the node id: {body}"
    );
    let body = json!({"name": "beta", "captain_node_id": "02GHOST"});
    let (status, _) = call(&app, "POST", "/api/teams", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn patch_captain_and_members_with_captain_removal_guard() {
    let env = env().await;
    let app = plain_app(&env, Arc::new(MockDispatcher::new()));
    let captain = register(&env, "cap-b").await;
    let member = register(&env, "mem-b").await;
    let other = register(&env, "oth-b").await;
    let extra = register(&env, "ext-b").await;
    make_team(&app, "bravo", &captain.id, std::slice::from_ref(&member.id)).await;

    // PATCH captain to `other` (registered node; need not be a member).
    let (status, body) = call(
        &app,
        "PATCH",
        "/api/teams/bravo",
        Some(json!({"captain_node_id": other.id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["team"]["captain"]["node_id"], other.id.as_str());

    // Unknown captain → 400; unknown team → 404.
    let (status, _) = call(
        &app,
        "PATCH",
        "/api/teams/bravo",
        Some(json!({"captain_node_id": "03GHOST"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "PATCH",
        "/api/teams/ghost-team",
        Some(json!({"captain_node_id": other.id})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // members add: `member` already there (idempotent), the ex-captain may
    // double as member, `other` joins → 3 rows.
    let body =
        json!({"add": [member.id.clone(), captain.id.clone(), other.id.clone()], "remove": []});
    let (status, body) = call(&app, "POST", "/api/teams/bravo/members", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["team"]["members"].as_array().unwrap().len(), 3);

    let body = json!({"add": [], "remove": [member.id.clone()]});
    let (status, body) = call(&app, "POST", "/api/teams/bravo/members", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["team"]["members"].as_array().unwrap().len(), 2);

    // Removing the CURRENT captain (`other` since the PATCH above) is the
    // one forbidden removal; removing the EX-captain is fine.
    let body = json!({"add": [], "remove": [other.id.clone()]});
    let (status, body) = call(&app, "POST", "/api/teams/bravo/members", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("captain"),
        "error must explain the captain guard: {body}"
    );
    let body = json!({"add": [], "remove": [captain.id.clone()]});
    let (status, body) = call(&app, "POST", "/api/teams/bravo/members", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["team"]["members"].as_array().unwrap().len(), 1);

    // Hand the captain's seat to a node outside the member list, then drop
    // the last member: an empty member list is legal (the captain stands
    // alone until someone is added).
    let body = json!({"captain_node_id": extra.id});
    let (status, _) = call(&app, "PATCH", "/api/teams/bravo", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    let body = json!({"add": [], "remove": [other.id.clone()]});
    let (status, body) = call(&app, "POST", "/api/teams/bravo/members", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["team"]["members"].as_array().unwrap().len(), 0);

    // Adding an unregistered node → 400 naming the node.
    let body = json!({"add": ["04GHOST"], "remove": []});
    let (status, body) = call(&app, "POST", "/api/teams/bravo/members", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("04GHOST"));
}

// ── topics ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn topic_runs_to_complete_and_writes_the_share_layout() {
    let env = env().await;
    let captain = register(&env, "captain-c").await;
    let member = register(&env, "member-c").await;
    let app = plain_app(&env, Arc::new(happy_script(&captain, &member)));
    make_team(&app, "delta", &captain.id, std::slice::from_ref(&member.id)).await;

    // Input validation before any spawn.
    let (status, _) = call(
        &app,
        "POST",
        "/api/teams/delta/topics",
        Some(json!({"title": "  ", "requirement": "r"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/api/teams/nope/topics",
        Some(json!({"title": "t", "requirement": "r"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Create → 201 executing.
    let (status, body) = call(
        &app,
        "POST",
        "/api/teams/delta/topics",
        Some(json!({"title": "布局讨论", "requirement": "确定目录结构"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let topic = body["topic"].as_object().unwrap();
    assert_eq!(topic["status"], "executing");
    assert_eq!(topic["team_name"], "delta");
    let tid = topic["topic_id"].as_str().unwrap().to_string();
    let detail_uri = format!("/api/teams/delta/topics/{tid}");

    // The runtime is spawned: poll detail until finished/complete.
    let detail = poll_until(&app, &detail_uri, Duration::from_secs(5), |b| {
        b["topic"]["status"] == "finished"
    })
    .await;
    assert_eq!(detail["topic"]["finish_reason"], "complete");
    assert!(
        detail["topic"]["final_summary"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "final_summary must exist: {detail}"
    );
    let turns = detail["turns"].as_array().unwrap();
    assert!(!turns.is_empty(), "turns must be recorded");
    assert!(turns[0]["plan"].is_object());
    assert_eq!(
        turns[0]["sub_turns"][0]["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // The team's topic list and the cross-team list both carry it.
    let (status, list) = call(&app, "GET", "/api/teams/delta/topics", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["topics"].as_array().unwrap().len(), 1);
    let (status, all) = call(&app, "GET", "/api/topics", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["topics"].as_array().unwrap().len(), 1);

    // Share layout: plan / result / summary files for turn 1 / sub-turn 0.
    let share = env.team_root.join("delta").join(&tid);
    assert!(share.join("1").join("plan.json").is_file());
    assert!(share
        .join("1")
        .join("0")
        .join(&member.id)
        .join("result.json")
        .is_file());
    assert!(share.join("1").join("0").join("summary.json").is_file());

    // Detail of an unknown topic → 404.
    let (status, _) = call(
        &app,
        "GET",
        "/api/teams/delta/topics/01ZZZZZZZZZZZZZZZZZZZZZZZZ",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── cancel ────────────────────────────────────────────────────────────────

/// A dispatcher whose asks linger before answering a valid plan decision:
/// the runtime is genuinely inside its first dispatch when the test cancels,
/// so the hub-hit path is exercised deterministically.
struct SlowPlanDispatcher {
    delay: Duration,
    reply: String,
    asks: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl TeamDispatcher for SlowPlanDispatcher {
    async fn ask(
        &self,
        _topic: Option<&str>,
        _node: &str,
        _prompt: &str,
    ) -> anyhow::Result<String> {
        self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(self.reply.clone())
    }
}

#[tokio::test]
async fn cancel_hits_the_running_runtime_through_its_token() {
    let env = env().await;
    let captain = register(&env, "captain-d").await;
    let member = register(&env, "member-d").await;
    let slow = Arc::new(SlowPlanDispatcher {
        delay: Duration::from_millis(800),
        reply: plan("慢问题", std::slice::from_ref(&member.id)),
        asks: std::sync::atomic::AtomicUsize::new(0),
    });
    let app = plain_app(&env, slow.clone());
    make_team(&app, "echo", &captain.id, std::slice::from_ref(&member.id)).await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/teams/echo/topics",
        Some(json!({"title": "取消我", "requirement": "r"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let tid = body["topic"]["topic_id"].as_str().unwrap().to_string();

    // Wait until the runtime is truly inside its first ask, then cancel.
    let start = Instant::now();
    while slow.asks.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "runtime never asked"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let uri = format!("/api/teams/echo/topics/{tid}/cancel");
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));

    // The cancelled token converges the topic to finished(cancelled) once
    // the in-flight ask returns and the loop-top check fires.
    let detail = poll_until(
        &app,
        &format!("/api/teams/echo/topics/{tid}"),
        Duration::from_secs(5),
        |b| b["topic"]["status"] == "finished",
    )
    .await;
    assert_eq!(detail["topic"]["finish_reason"], "cancelled");

    // Cancelling again on a terminal topic is idempotently ok:true.
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));
}

#[tokio::test]
async fn cancel_without_a_runtime_converges_the_disk_state() {
    let env = env().await;
    let app = plain_app(&env, Arc::new(MockDispatcher::new()));

    // An executing topic left behind by a previous server process: no hub
    // entry, so the endpoint must converge it to finished(cancelled).
    let tid = hand_topic(&env.team_root, "orphan", "executing", None, 1_000);
    let uri = format!("/api/teams/orphan/topics/{tid}/cancel");
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let detail_uri = format!("/api/teams/orphan/topics/{tid}");
    let (status, detail) = call(&app, "GET", &detail_uri, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["topic"]["status"], "finished");
    assert_eq!(detail["topic"]["finish_reason"], "cancelled");

    // finished(error) topics are converged by cancel too (they are the
    // resume candidates; cancelling one gives up on it).
    let tid2 = hand_topic(&env.team_root, "orphan", "finished", Some("error"), 2_000);
    let uri2 = format!("/api/teams/orphan/topics/{tid2}/cancel");
    let (status, _) = call(&app, "POST", &uri2, None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, detail2) = call(
        &app,
        "GET",
        &format!("/api/teams/orphan/topics/{tid2}"),
        None,
    )
    .await;
    assert_eq!(detail2["topic"]["finish_reason"], "cancelled");

    // Unknown topic → 404.
    let (status, _) = call(
        &app,
        "POST",
        "/api/teams/orphan/topics/01QQQQQQQQQQQQQQQQQQQQQQQQ/cancel",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── resume ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn resume_accepts_error_topics_and_rejects_terminal_ones() {
    let env = env().await;
    let captain = register(&env, "captain-e").await;
    let member = register(&env, "member-e").await;
    hand_team(&env.team_root, &captain, &member, "foxtrot");

    // finished(error) with no turns → resume re-runs from the plan stage;
    // the scripted captain completes the whole discussion this time.
    let tid = hand_topic(&env.team_root, "foxtrot", "finished", Some("error"), 1_000);
    {
        let mut meta = fs_store::load_topic(&env.team_root, "foxtrot", &tid).unwrap();
        meta.captain = MemberRef {
            node_id: captain.id.clone(),
            name: captain.name.clone(),
        };
        meta.members = vec![MemberRef {
            node_id: member.id.clone(),
            name: member.name.clone(),
        }];
        fs_store::save_topic(&env.team_root, &meta).unwrap();
    }

    let app = plain_app(&env, Arc::new(happy_script(&captain, &member)));
    let uri = format!("/api/teams/foxtrot/topics/{tid}/resume");
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["accepted"], json!(true));

    let detail = poll_until(
        &app,
        &format!("/api/teams/foxtrot/topics/{tid}"),
        Duration::from_secs(5),
        |b| b["topic"]["status"] == "finished",
    )
    .await;
    assert_eq!(detail["topic"]["finish_reason"], "complete");

    // Resuming the now-terminal topic → 409.
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // finished(complete) from the start → 409; unknown topic → 404.
    let tid2 = hand_topic(
        &env.team_root,
        "foxtrot",
        "finished",
        Some("complete"),
        2_000,
    );
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/teams/foxtrot/topics/{tid2}/resume"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &app,
        "POST",
        "/api/teams/foxtrot/topics/01QQQQQQQQQQQQQQQQQQQQQQQQ/resume",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// M2' regression: `terminal::finish` saves the terminal metadata before
// flipping the ledger rows; a crash in between leaves `disk
// finished(complete) + ledger executing`, and the runtime's own converge
// branch is unreachable (resume 409s before spawning). The 409 rejection
// itself must converge the rows — any resume retry clears the residue.
#[tokio::test]
async fn resume_rejection_converges_stale_executing_ledger() {
    let env = env().await;
    let captain = register(&env, "captain-l").await;
    let member = register(&env, "member-l").await;
    hand_team(&env.team_root, &captain, &member, "lima");
    let tid = hand_topic(&env.team_root, "lima", "finished", Some("complete"), 1_000);
    // Crash residue: the pairing row never got flipped.
    env.store
        .upsert_team_topic_run(&TeamTopicRunRecord {
            topic_id: tid.clone(),
            node_id: member.id.clone(),
            status: TEAM_RUN_EXECUTING.to_string(),
            created_at: 1_234,
        })
        .await
        .unwrap();

    let app = plain_app(&env, Arc::new(MockDispatcher::new()));
    let uri = format!("/api/teams/lima/topics/{tid}/resume");
    let (status, body) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let runs = env.store.list_team_topic_runs(&tid).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TEAM_RUN_FINISHED);

    // Idempotent: retrying the rejection keeps the ledger converged.
    let (status, _) = call(&app, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let runs = env.store.list_team_topic_runs(&tid).await.unwrap();
    assert!(runs.iter().all(|r| r.status == TEAM_RUN_FINISHED));
}

// ── cross-team listing ────────────────────────────────────────────────────

#[tokio::test]
async fn all_topics_listing_and_team_filter() {
    let env = env().await;
    let app = plain_app(&env, Arc::new(MockDispatcher::new()));
    let t_old = hand_topic(&env.team_root, "golf", "executing", None, 1_000);
    let t_new = hand_topic(&env.team_root, "hotel", "finished", Some("complete"), 9_000);
    hand_topic(&env.team_root, "golf", "finished", Some("cancelled"), 5_000);

    let (status, all) = call(&app, "GET", "/api/topics", None).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let topics = all["topics"].as_array().unwrap();
    assert_eq!(topics.len(), 3, "{all}");
    assert_eq!(topics[0]["topic_id"], json!(t_new), "newest first");
    assert_eq!(topics[1]["created_at"], json!(5_000));
    assert_eq!(topics[2]["topic_id"], json!(t_old));

    let (status, one) = call(&app, "GET", "/api/topics?team=golf", None).await;
    assert_eq!(status, StatusCode::OK, "{one}");
    let filtered = one["topics"].as_array().unwrap();
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().all(|t| t["team_name"] == "golf"));

    // Invalid team name in the filter → 400 (never a path traversal).
    let (status, _) = call(&app, "GET", "/api/topics?team=..%2Fetc", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Listing the topics of an unknown team → 404.
    let (status, _) = call(&app, "GET", "/api/teams/ghost/topics", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── profiling ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn profile_accepts_and_fills_capabilities_in_the_background() {
    let env = env().await;
    let captain = register(&env, "captain-f").await;
    let member = register(&env, "member-f").await;
    let scripted = MockDispatcher::new().reply(
        &member.id,
        vec![ok(
            json!({"capabilities": ["Rust 后端", "SQLite"]}).to_string()
        )],
    );
    let app = plain_app(&env, Arc::new(scripted));
    make_team(&app, "india", &captain.id, std::slice::from_ref(&member.id)).await;

    let (status, body) = call(&app, "POST", "/api/teams/india/profile", None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["accepted"], json!(true));

    // profile_team rewrites team.json off-thread: poll until the member's
    // capabilities + profiled_at appear.
    let list = poll_until(&app, "/api/teams", Duration::from_secs(5), |b| {
        b["teams"]
            .as_array()
            .and_then(|t| t.first())
            .and_then(|t| t["members"].as_array())
            .and_then(|m| m.first())
            .and_then(|m| m["capabilities"].as_array())
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    })
    .await;
    let member_row = &list["teams"][0]["members"][0];
    assert_eq!(member_row["capabilities"], json!(["Rust 后端", "SQLite"]));
    assert!(member_row["profiled_at"].is_i64());

    // Profiling an unknown team → 404.
    let (status, _) = call(&app, "POST", "/api/teams/ghost/profile", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
