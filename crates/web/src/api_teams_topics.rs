//! Topic half of the team HTTP surface (`/api/teams/:name/topics*`,
//! `/api/topics`): create / list / detail / cancel / resume. Split from
//! `api_teams.rs` for the file-size budget; `api_teams::routes()` wires both.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use opencoder_core::message::now_ms;
use opencoder_team::fs_store;
use opencoder_team::layout;
use opencoder_team::types::{
    TopicMeta, FINISH_CANCELLED, FINISH_ERROR, TOPIC_EXECUTING, TOPIC_FINISHED,
};
use opencoder_team::validate_team_name;

use crate::api::{error_400, error_404, error_409, error_500};
use crate::team_hub::spawn_topic_runtime;
use crate::AppState;

// ── topics ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateTopicBody {
    pub title: String,
    pub requirement: String,
}

/// POST /api/teams/:name/topics — snapshot the team into a fresh executing
/// topic (`start_topic` re-verifies every node registration) and spawn its
/// runtime; the response already carries the initial metadata.
pub async fn create_topic(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<CreateTopicBody>,
) -> Response {
    if body.title.trim().is_empty() {
        return error_400("title must not be empty".into());
    }
    if body.requirement.trim().is_empty() {
        return error_400("requirement must not be empty".into());
    }
    let cfg = state.team.run.clone();
    if fs_store::load_team(&cfg.team_root, &name).is_err() {
        return error_404(&format!("team {name} not found"));
    }
    let meta = match opencoder_team::start_topic(
        state.store.clone(),
        &cfg,
        &name,
        body.title.trim(),
        body.requirement.trim(),
    )
    .await
    {
        Ok(meta) => meta,
        Err(e) => return error_400(format!("start_topic: {e:#}")),
    };
    let topic_id = meta.topic_id.clone();
    spawn_topic_runtime(state, name, topic_id);
    (StatusCode::CREATED, Json(json!({ "topic": meta }))).into_response()
}

/// GET /api/teams/:name/topics — the team's topics, newest first.
pub async fn list_team_topics(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    let root = &state.team.run.team_root;
    if fs_store::load_team(root, &name).is_err() {
        return error_404(&format!("team {name} not found"));
    }
    Json(json!({ "topics": collect_topics(root, &name) })).into_response()
}

/// GET /api/teams/:name/topics/:tid — full discussion tree (turns, plans,
/// per-member results, summaries) beside the topic metadata.
pub async fn get_topic(
    State(state): State<Arc<AppState>>,
    Path((name, tid)): Path<(String, String)>,
) -> Response {
    let root = &state.team.run.team_root;
    match fs_store::read_topic_tree(root, &name, &tid) {
        Ok((topic, turns)) => Json(json!({ "topic": topic, "turns": turns })).into_response(),
        Err(_) => error_404(&format!("topic {tid} not found")),
    }
}

/// POST /api/teams/:name/topics/:tid/cancel — idempotent. A live runtime is
/// cancelled through its token (the runtime persists the terminal state);
/// a topic with no in-process runtime (server restarted, or finished with
/// `error`) is converged to `finished(cancelled)` right here.
pub async fn cancel_topic(
    State(state): State<Arc<AppState>>,
    Path((name, tid)): Path<(String, String)>,
) -> Response {
    let root = &state.team.run.team_root;
    if state.team.hub.cancel(&tid) {
        return Json(json!({ "ok": true })).into_response();
    }
    let mut meta = match fs_store::load_topic(root, &name, &tid) {
        Ok(meta) => meta,
        Err(_) => return error_404(&format!("topic {tid} not found")),
    };
    if meta.status == TOPIC_EXECUTING || meta.finish_reason.as_deref() == Some(FINISH_ERROR) {
        meta.status = TOPIC_FINISHED.to_string();
        meta.finish_reason = Some(FINISH_CANCELLED.to_string());
        meta.finished_at = Some(now_ms());
        if let Err(e) = fs_store::save_topic(root, &meta) {
            return error_500(format!("save_topic: {e:#}"));
        }
        if let Err(e) = state.store.finish_team_topic_run(&tid).await {
            return error_500(format!("finish_team_topic_run: {e:#}"));
        }
    }
    Json(json!({ "ok": true })).into_response()
}

/// POST /api/teams/:name/topics/:tid/resume — 202 + spawned `run_topic`
/// (resume is derived from the on-disk tree). Only `executing` orphans (a
/// server restart left them mid-run) and `finished(error)` topics may
/// resume; anything else answers 409, as does an already-running topic.
/// That 409 first converges the topic's ledger rows (idempotent flip): the
/// runtime saves terminal metadata BEFORE flipping the store rows, and this
/// rejection is the only production entry left for that crash residue.
pub async fn resume_topic(
    State(state): State<Arc<AppState>>,
    Path((name, tid)): Path<(String, String)>,
) -> Response {
    let root = &state.team.run.team_root;
    let meta = match fs_store::load_topic(root, &name, &tid) {
        Ok(meta) => meta,
        Err(_) => return error_404(&format!("topic {tid} not found")),
    };
    if state.team.hub.is_running(&tid) {
        return error_409("topic runtime is already running");
    }
    let resumable =
        meta.status == TOPIC_EXECUTING || meta.finish_reason.as_deref() == Some(FINISH_ERROR);
    if !resumable {
        // Crash residue `disk finished + ledger executing` can never reach
        // `run_topic`'s own converge branch (we refuse before spawning), so
        // converge the idempotent flip right here on the rejection path.
        if let Err(e) = state.store.finish_team_topic_run(&tid).await {
            return error_500(format!("finish_team_topic_run: {e:#}"));
        }
        return error_409(&format!(
            "topic is {} ({}); only executing or finished(error) topics can resume",
            meta.status,
            meta.finish_reason.unwrap_or_default()
        ));
    }
    // Flip the topic to `executing` BEFORE spawning so the 202 reflects disk
    // truth: a client that immediately GETs the detail (or re-resumes after
    // the hub token is gone) must not still see the terminal state.
    let mut meta = meta;
    meta.status = TOPIC_EXECUTING.to_string();
    meta.finish_reason = None;
    meta.finished_at = None;
    if let Err(e) = fs_store::save_topic(root, &meta) {
        return error_500(format!("save_topic: {e:#}"));
    }
    spawn_topic_runtime(state, name, tid);
    (StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response()
}

// ── cross-team listing ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TopicsQuery {
    pub team: Option<String>,
}

/// GET /api/topics?team= — every topic across teams (optionally one team's),
/// newest first. Plain `TopicMeta` list; the SPA derives anything else
/// client-side (Phase 4 contract).
pub async fn list_all_topics(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TopicsQuery>,
) -> Response {
    let root = &state.team.run.team_root;
    let team_names: Vec<String> = match q.team {
        Some(name) => {
            if !validate_team_name(&name) {
                return error_400(format!("invalid team name {name:?}"));
            }
            vec![name]
        }
        None => layout::list_team_dirs(root).unwrap_or_default(),
    };
    let mut topics = Vec::new();
    for team in team_names {
        topics.extend(collect_topics(root, &team));
    }
    topics.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(b.topic_id.cmp(&a.topic_id))
    });
    Json(json!({ "topics": topics })).into_response()
}

/// One team's topics, newest first; unreadable topic dirs are skipped with
/// a warning (a half-written share must not break listing).
fn collect_topics(root: &std::path::Path, team_name: &str) -> Vec<TopicMeta> {
    let dir = match layout::team_dir(root, team_name) {
        Ok(dir) => dir,
        Err(e) => {
            tracing::warn!(team = %team_name, error = %format!("{e:#}"), "topic dir unusable");
            return Vec::new();
        }
    };
    let mut topics = Vec::new();
    for tid in layout::list_topic_dirs(&dir).unwrap_or_default() {
        match fs_store::load_topic(root, team_name, &tid) {
            Ok(meta) => topics.push(meta),
            Err(e) => {
                tracing::warn!(team = %team_name, topic = %tid, error = %format!("{e:#}"), "skipping unreadable topic")
            }
        }
    }
    topics.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(b.topic_id.cmp(&a.topic_id))
    });
    topics
}
