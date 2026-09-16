//! HTTP e2e for the topic-progression contract: one team, the captain opens
//! TWO topics in sequence over the scripted 19-prompt conversation. Topic 1
//! needs a called-out clarification sub-turn + a second round before it
//! completes; topic 2 completes in one aligned round. Verified through the
//! detail tree (turns / sub_turns / results / summaries), the cross-team
//! listing, the share layout, and the concatenated final conclusion.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::Request;
use opencoder_store::NodeRecord;
use opencoder_team::{ok, MockDispatcher, TeamDispatcher, TeamRunConfig};
use opencoder_web::team_state::TeamWebState;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

// ── harness (mirrors api_teams.rs) ────────────────────────────────────────

struct Env {
    store: Arc<dyn opencoder_store::Store>,
    team_root: PathBuf,
    _root: TempDir,
    _db: TempDir,
}

async fn env() -> Env {
    let db = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let store: Arc<dyn opencoder_store::Store> = Arc::new(
        opencoder_store::LibsqlStore::open(db.path().join("t.db"))
            .await
            .unwrap(),
    );
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

fn app_for(env: &Env, dispatcher: Arc<dyn TeamDispatcher>) -> axum::Router {
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
    opencoder_web::build_app(state, None, false)
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (axum::http::StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(json) => b
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
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

async fn poll_until<F>(app: &axum::Router, uri: &str, pred: F) -> Value
where
    F: Fn(&Value) -> bool,
{
    let start = Instant::now();
    loop {
        let (status, body) = call(app, "GET", uri, None).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        if pred(&body) {
            return body;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out waiting for {uri}; last: {body}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── decision-JSON builders ────────────────────────────────────────────────

fn plan(question: &str, participants: &[String]) -> String {
    json!({"question": question, "participants": participants, "rationale": "理由"}).to_string()
}

fn summary(text: &str, aligned: bool, ambiguities: &[(&str, &str)]) -> String {
    json!({
        "summary": text,
        "aligned": aligned,
        "ambiguities": ambiguities
            .iter()
            .map(|(node_id, question)| json!({"node_id": node_id, "question": question}))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

fn closing_complete(final_summary: &str) -> String {
    json!({"complete": true, "next_question": null, "final_summary": final_summary}).to_string()
}

fn closing_continue(next_question: &str) -> String {
    json!({"complete": false, "next_question": next_question, "final_summary": null}).to_string()
}

/// The same two-topic script the team crate's own contract test drives.
fn script(
    captain: &NodeRecord,
    backend: &NodeRecord,
    sre: &NodeRecord,
    risk: &NodeRecord,
) -> MockDispatcher {
    let all: Vec<String> = [backend, sre, risk].iter().map(|m| m.id.clone()).collect();
    MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok(plan("回调超时阈值设多少", &all)),
                ok(summary(
                    "超时阈值与降级口径未对齐",
                    false,
                    &[
                        (&backend.id, "渠道真实 P99 是多少？请给出数据依据"),
                        (&risk.id, "降级放行的开关归属谁"),
                    ],
                )),
                ok(summary("超时上限取 3.5s，超时降级放行", true, &[])),
                ok(closing_continue("降级放行的具体形态")),
                ok(plan(
                    "降级后如何放行与记账",
                    &[backend.id.clone(), risk.id.clone()],
                )),
                ok(summary("降级放行与记账口径一致", true, &[])),
                ok(closing_complete(
                    "回调超时按渠道 P99 2.4s 的 1.5 倍取 3.5s，超时降级放行并标记事后回捞",
                )),
                ok(plan(
                    "失败后的重试与幂等",
                    &[backend.id.clone(), sre.id.clone()],
                )),
                ok(summary("重试 3 次加幂等键的方案一致", true, &[])),
                ok(closing_complete(
                    "失败重试 3 次（指数退避），以 request_no 作幂等键去重",
                )),
            ],
        )
        .reply(
            &backend.id,
            vec![
                ok("我建议同步等待 5s 超时"),
                ok("对齐：渠道实测 P99 2.4s，按 1.5 倍取 3.5s"),
                ok("降级放行：先落库标记 degraded，事后补偿"),
                ok("重试 3 次指数退避，幂等键用 request_no"),
            ],
        )
        .reply(
            &sre.id,
            vec![
                ok("渠道 P99 2.4s，5s 太长，建议 3.5s"),
                ok("重试 3 次可接受，必须幂等防重"),
            ],
        )
        .reply(
            &risk.id,
            vec![
                ok("超时必须降级放行，不能阻塞主链路"),
                ok("对齐：降级开关由风控平台持有，回调侧只执行"),
                ok("放行必须携带风控降级标记"),
            ],
        )
}

// ── the scenario ──────────────────────────────────────────────────────────

#[tokio::test]
async fn team_topic_sequence_drives_two_topics_to_a_final_conclusion() {
    let env = env().await;
    let captain = register(&env, "arch-captain").await;
    let backend = register(&env, "pay-backend").await;
    let sre = register(&env, "sre").await;
    let risk = register(&env, "risk-ctl").await;
    let app = Arc::new(app_for(
        &env,
        Arc::new(script(&captain, &backend, &sre, &risk)),
    ));
    let team = "pay-alignment";

    let (status, body) = call(
        &app,
        "POST",
        "/api/teams",
        Some(json!({
            "name": team,
            "captain_node_id": captain.id,
            "member_node_ids": [&backend.id, &sre.id, &risk.id],
        })),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{body}");

    // ── topic 1: plan → disagree → called-out follow-up → turn 2 → close ──
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/topics"),
        Some(json!({"title": "回调超时与降级", "requirement": "确定支付回调的超时阈值与降级策略"})),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{body}");
    let tid1 = body["topic"]["topic_id"].as_str().unwrap().to_string();
    let detail1 = poll_until(&app, &format!("/api/teams/{team}/topics/{tid1}"), |b| {
        b["topic"]["status"] == "finished"
    })
    .await;
    assert_eq!(detail1["topic"]["finish_reason"], "complete");
    let turns1 = detail1["turns"].as_array().unwrap();
    assert_eq!(turns1.len(), 2);
    assert_eq!(turns1[0]["sub_turns"].as_array().unwrap().len(), 2);
    assert_eq!(turns1[1]["sub_turns"].as_array().unwrap().len(), 1);
    assert_eq!(
        turns1[0]["sub_turns"][1]["results"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "only backend + risk answer the follow-up"
    );
    assert_eq!(
        turns1[1]["plan"]["question"], "降级后如何放行与记账",
        "turn 2 shrinks the participants"
    );
    assert_eq!(
        turns1[1]["sub_turns"][0]["results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // ── topic 2: a NEW topic, one aligned round ───────────────────────────
    let (status, body) = call(
        &app,
        "POST",
        &format!("/api/teams/{team}/topics"),
        Some(json!({"title": "重试与幂等", "requirement": "确定回调失败后的重试与幂等方案"})),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{body}");
    let tid2 = body["topic"]["topic_id"].as_str().unwrap().to_string();
    let detail2 = poll_until(&app, &format!("/api/teams/{team}/topics/{tid2}"), |b| {
        b["topic"]["status"] == "finished"
    })
    .await;
    assert_eq!(detail2["topic"]["finish_reason"], "complete");
    let turns2 = detail2["turns"].as_array().unwrap();
    assert_eq!(turns2.len(), 1);
    assert_eq!(turns2[0]["sub_turns"].as_array().unwrap().len(), 1);

    // ── listings: the team's two topics, newest first across teams ────────
    let (status, list) = call(&app, "GET", &format!("/api/teams/{team}/topics"), None).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(list["topics"].as_array().unwrap().len(), 2);
    let (status, all) = call(&app, "GET", "/api/topics", None).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let listed = all["topics"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["topic_id"], json!(tid2), "newest first");

    // ── share layout: follow-up answers live only under 1/1/<node> ────────
    let share = env.team_root.join(team);
    let t1 = share.join(&tid1);
    assert!(t1.join("1").join("plan.json").is_file());
    assert!(t1
        .join("1")
        .join("1")
        .join(&backend.id)
        .join("result.json")
        .is_file());
    assert!(t1
        .join("1")
        .join("1")
        .join(&risk.id)
        .join("result.json")
        .is_file());
    assert!(
        !t1.join("1").join("1").join(&sre.id).exists(),
        "sre not re-asked"
    );
    assert!(t1.join("1").join("0").join("summary.json").is_file());
    assert!(t1.join("1").join("1").join("summary.json").is_file());
    assert!(t1.join("2").join("0").join("summary.json").is_file());
    assert!(share
        .join(&tid2)
        .join("1")
        .join("0")
        .join("summary.json")
        .is_file());

    // ── the deliverable conclusion concatenates both final summaries ─────
    let text1 = detail1["topic"]["final_summary"].as_str().unwrap();
    let text2 = detail2["topic"]["final_summary"].as_str().unwrap();
    assert!(text1.contains("2.4s") && text1.contains("放行"));
    assert!(text2.contains("重试 3 次") && text2.contains("幂等键"));
    let conclusion = format!("{text1}{text2}");
    for marker in ["2.4s", "放行", "重试 3 次", "幂等键"] {
        assert!(conclusion.contains(marker), "conclusion misses {marker}");
    }
}
