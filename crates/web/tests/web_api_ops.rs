//! Functional tests for the new feature-parity endpoints:
//! fork, skill, config, compact, handoff, bg.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, patch, post};
use axum::Router;
use tower::ServiceExt;

use opencoder_core::Message;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

fn app(state: Arc<opencoder_web::AppState>) -> Router {
    Router::new()
        .route(
            "/api/sessions",
            post(opencoder_web::api::create_session).get(opencoder_web::api::list_sessions),
        )
        .route(
            "/api/sessions/:id",
            get(opencoder_web::api::get_session).delete(opencoder_web::api::delete_session),
        )
        .route(
            "/api/sessions/:id/fork",
            post(opencoder_web::api_ops::fork_session),
        )
        .route(
            "/api/sessions/:id/skill",
            post(opencoder_web::api_ops::post_skill),
        )
        .route(
            "/api/sessions/:id/compact",
            post(opencoder_web::api_ops::post_compact),
        )
        .route(
            "/api/sessions/:id/handoff",
            post(opencoder_web::api_ops::post_handoff),
        )
        .route("/api/config", get(opencoder_web::api_ops::get_config))
        .route("/api/config", patch(opencoder_web::api_ops::patch_config))
        .route("/api/bg", get(opencoder_web::api_ops::list_bg))
        .route("/api/bg/stop", post(opencoder_web::api_ops::stop_bg))
        .route("/api/health", get(opencoder_web::api::health))
        .with_state(state)
}

async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-ops-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).ok();
    Arc::new(opencoder_web::AppState {
        client_override: Some(Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
    })
}

/// AppState whose drain mock returns a deterministic completion — needed for
/// endpoints whose drain task calls the LLM (e.g. manual compaction).
async fn state_with_reply(text: &str) -> Arc<opencoder_web::AppState> {
    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = std::env::temp_dir().join(format!("oc-web-ops-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).ok();
    Arc::new(opencoder_web::AppState {
        client_override: Some(mock),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
    })
}

async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.to_string(),
            title: Some("test".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id.to_string());
    m.blocks.push(opencoder_core::ContentBlock::text(text));
    m
}

#[tokio::test]
async fn fork_copies_messages_and_returns_new_id() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "parent").await;
    state
        .store
        .append_messages(
            "parent",
            &[
                Message::user("u1".to_string(), "hello"),
                assistant_with_text("a1", "hi there"),
            ],
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/parent/fork")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let new_id = v["id"].as_str().expect("id");
    assert_ne!(new_id, "parent");

    let msgs = state.store.load_messages(new_id).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].text(), "hello");
    assert_eq!(msgs[1].text(), "hi there");

    let parent = state.store.load_messages("parent").await.unwrap();
    assert_eq!(parent.len(), 2, "parent unchanged");
}

#[tokio::test]
async fn fork_nonexistent_returns_404() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/nope/fork")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn fork_title_gets_fork_suffix() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "p2").await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/p2/fork")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let meta = state
        .store
        .get_session(v["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.title.as_deref(), Some("test (fork)"));
}

#[tokio::test]
async fn skill_persists_to_store_meta() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "s1").await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/s1/skill")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"skill":"repo-local-memory"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = state.store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(meta.skill.as_deref(), Some("repo-local-memory"));
}

#[tokio::test]
async fn skill_clear_with_null() {
    let state = state().await;
    let app = app(state.clone());
    seed(&state, "s2").await;
    state
        .store
        .update_session(
            "s2",
            &opencoder_store::SessionPatch {
                skill: Some("my-skill".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/s2/skill")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"skill":null}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let meta = state.store.get_session("s2").await.unwrap().unwrap();
    assert!(meta.skill.is_none(), "skill should be cleared");
}

#[tokio::test]
async fn skill_nonexistent_returns_404() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/nope/skill")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"skill":"repo-local-memory"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    // post_skill must reject unknown sessions the same way post_compact /
    // post_handoff do — previously it returned a false `{ok:true}` success.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], serde_json::Value::Bool(false));
    assert!(
        v["error"].as_str().unwrap().contains("session not found"),
        "expected 'session not found' error, got: {v}"
    );
}

#[tokio::test]
async fn patch_config_writes_skills_domain_file() {
    let state = state().await;
    // Isolate config discovery: the skills domain write must land under the
    // scoped home, never in the real ~/.opencoder.
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    let app = app(state.clone());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"skills":{"alpha":{"enabled":true}}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], serde_json::Value::Bool(true));

    // The `skills` patch lands in the dedicated domain file. Its top level
    // IS the per-skill map — no `skills` wrapper key.
    let skills_path = state.workdir.join(".opencoder").join("skills.json");
    let domain: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&skills_path).unwrap()).unwrap();
    assert_eq!(domain["alpha"]["enabled"], true);
    assert_eq!(domain.as_object().map(|o| o.len()), Some(1));

    // config.json must not carry the domain key (a domain-only patch must
    // not create it at all; if some other path did, the key must be absent).
    for cfg_path in [
        state.workdir.join(".opencoder").join("config.json"),
        state.workdir.join("opencoder.json"),
    ] {
        if cfg_path.exists() {
            let cfg: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
            assert!(
                cfg.get("skills").is_none(),
                "{} must not contain a `skills` key: {cfg}",
                cfg_path.display()
            );
        }
    }

    // A follow-up GET reflects the persisted toggle (config load path).
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        v["skills"]["alpha"]["enabled"], true,
        "GET /api/config must reflect the domain-file toggle"
    );
}

#[tokio::test]
async fn get_config_returns_json() {
    let state = state().await;
    // Isolate config discovery so this test never reads the real
    // ~/.opencoder/config.json (host secrets/values must never leak in).
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v.is_object());
    assert!(v.get("model").is_some());
}

#[tokio::test]
async fn patch_config_merges_and_persists() {
    let state = state().await;
    // Isolate config discovery so save_target never escapes to the real
    // ~/.opencoder/config.json on a host where HOME=/root.
    let _iso = opencoder_core::scoped_config_home(state.workdir.clone());
    // Derive the test model from the loaded config instead of hardcoding a
    // magic string — the test stays deterministic without coupling to a
    // specific provider value that could clash with a real config on disk.
    let before = opencoder_core::Config::load(&state.workdir).unwrap();
    let new_model = format!("test-{}", before.model);
    let body = format!(r#"{{"model":"{}"}}"#, new_model);
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cfg = opencoder_core::Config::load(&state.workdir).unwrap();
    assert_eq!(cfg.model, new_model);
}

#[tokio::test]
async fn list_bg_returns_empty_array() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/bg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["processes"].is_array());
}

#[tokio::test]
async fn stop_bg_returns_ok() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/bg/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["ok"].as_bool().unwrap());
}

#[tokio::test]
async fn compact_returns_ok_and_persists_summary() {
    let state = state_with_reply("conversation summary text").await;
    let app = app(state.clone());
    let sid = "c1";
    seed(&state, sid).await;
    // Seed >=2 turns so compaction_split produces a non-empty head to summarize.
    state
        .store
        .append_messages(
            sid,
            &[
                Message::user("u1", "what is rust?"),
                assistant_with_text("a1", "a systems programming language"),
                Message::user("u2", "show me an example"),
                assistant_with_text("a2", "fn main() { println!(\"hi\"); }"),
            ],
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/c1/compact")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // The Compact command is processed asynchronously by the drain task.
    // Poll the durable store for the compaction boundary instead of a sleep.
    let mut compacted = false;
    for _ in 0..200 {
        if let Some(meta) = state.store.get_session(sid).await.unwrap() {
            if meta.summary_seq.is_some() {
                assert!(
                    meta.summary.is_some(),
                    "summary text must be persisted alongside summary_seq"
                );
                assert!(
                    meta.handoff_seq.is_none(),
                    "compaction must clear prior handoff state"
                );
                compacted = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        compacted,
        "manual compact must persist summary_seq to the store"
    );
}

#[tokio::test]
async fn compact_nonexistent_returns_404() {
    let state = state().await;
    let app = app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/nope/compact")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handoff_persists_boundary_when_plan_exists() {
    let state = state().await;
    let app = app(state.clone());
    let sid = "h1";
    seed(&state, sid).await;
    state
        .store
        .append_messages(
            sid,
            &[assistant_with_text("a1", "## Plan\n1. do X\n2. do Y")],
        )
        .await
        .unwrap();
    // Phase-bounded handoff (`plan_handoff::handoff`): the plan comes from
    // the snapshot `record` persists while the plan agent answers; the drain
    // restores it from the sessions row via `resume`. Seed that mirror
    // exactly as a real plan-mode session would have left it.
    state
        .store
        .update_session(
            sid,
            &opencoder_store::SessionPatch {
                plan_snapshot: Some("## Plan\n1. do X\n2. do Y".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/h1/handoff")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"extra":"begin"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // The Handoff command is processed asynchronously by the drain task.
    // Poll the durable store for the handoff boundary (handoff_seq + plan).
    let mut handed_off = false;
    for _ in 0..200 {
        if let Some(meta) = state.store.get_session(sid).await.unwrap() {
            if meta.handoff_seq.is_some() {
                assert!(
                    meta.handoff_plan.is_some(),
                    "handoff_plan must be persisted alongside handoff_seq"
                );
                assert_eq!(
                    meta.agent.as_deref(),
                    Some("act"),
                    "handoff switches the agent to act"
                );
                handed_off = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        handed_off,
        "manual handoff must persist handoff_seq to the store"
    );
}
