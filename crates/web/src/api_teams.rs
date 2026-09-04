//! Team HTTP surface (`/api/teams*`): registry listing, create, captain
//! handover, membership add/remove and background capability profiling.
//! Topic orchestration (create / list / detail / cancel / resume,
//! `/api/topics`) lives in [`crate::api_teams_topics`]; both halves are wired
//! by [`routes`]. Handlers are pure composition over `opencode_team` — the
//! run/turn semantics live in that crate; this layer only maps outcomes onto
//! HTTP statuses (400 invalid input · 404 unknown team · 409 conflict).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use opencoder_core::message::now_ms;
use opencoder_team::fs_store;
use opencoder_team::layout;
use opencoder_team::types::{MemberRef, TeamMember, TeamMeta};
use opencoder_team::validate_team_name;

use crate::api::{error_400, error_404, error_409, error_500};
use crate::api_teams_topics as topics;
use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/teams", get(list_teams).post(create_team))
        .route("/api/teams/:name", patch(patch_team))
        .route("/api/teams/:name/members", post(patch_members))
        .route("/api/teams/:name/profile", post(profile))
        .route(
            "/api/teams/:name/topics",
            get(topics::list_team_topics).post(topics::create_topic),
        )
        .route("/api/teams/:name/topics/:tid", get(topics::get_topic))
        .route(
            "/api/teams/:name/topics/:tid/cancel",
            post(topics::cancel_topic),
        )
        .route(
            "/api/teams/:name/topics/:tid/resume",
            post(topics::resume_topic),
        )
        .route("/api/topics", get(topics::list_all_topics))
}

// ── teams ─────────────────────────────────────────────────────────────────

/// GET /api/teams — every team with a readable team.json (corrupt ones are
/// skipped by `fs_store::list_teams` with a warning).
pub async fn list_teams(State(state): State<Arc<AppState>>) -> Response {
    Json(json!({ "teams": fs_store::list_teams(&state.team.run.team_root) })).into_response()
}

#[derive(Deserialize)]
pub struct CreateTeamBody {
    pub name: String,
    pub captain_node_id: String,
    #[serde(default)]
    pub member_node_ids: Vec<String>,
}

/// POST /api/teams — validate name + every referenced node, then persist.
/// Captain may double as a member (single row after dedup).
pub async fn create_team(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTeamBody>,
) -> Response {
    let root = &state.team.run.team_root;
    if !validate_team_name(&body.name) {
        return error_400(format!("invalid team name {:?}", body.name));
    }
    if layout::team_file(root, &body.name)
        .map(|p| p.exists())
        .unwrap_or(false)
    {
        return error_409(&format!("team {} already exists", body.name));
    }
    let captain = match state.store.get_node(&body.captain_node_id).await {
        Ok(Some(rec)) => rec,
        Ok(None) => {
            return error_400(format!(
                "captain node {} is not registered",
                body.captain_node_id
            ))
        }
        Err(e) => return error_500(format!("get_node: {e:#}")),
    };
    let mut member_ids = dedup(body.member_node_ids);
    member_ids.retain(|id| id != &body.captain_node_id);
    let mut members = Vec::new();
    for id in member_ids {
        match state.store.get_node(&id).await {
            Ok(Some(rec)) => members.push(TeamMember {
                node_id: rec.id,
                name: rec.name,
                capabilities: Vec::new(),
                profiled_at: None,
            }),
            Ok(None) => return error_400(format!("member node {id} is not registered")),
            Err(e) => return error_500(format!("get_node: {e:#}")),
        }
    }
    let now = now_ms();
    let meta = TeamMeta {
        name: body.name,
        captain: MemberRef {
            node_id: captain.id,
            name: captain.name,
        },
        members,
        created_at: now,
        updated_at: now,
    };
    match fs_store::create_team(root, &meta) {
        Ok(()) => (StatusCode::CREATED, Json(json!({ "team": meta }))).into_response(),
        Err(e) => error_500(format!("create_team: {e:#}")),
    }
}

#[derive(Deserialize)]
pub struct PatchTeamBody {
    pub captain_node_id: String,
}

/// PATCH /api/teams/:name — hand the captain role to another registered
/// node. Membership is untouched (the new captain need not be a member).
pub async fn patch_team(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<PatchTeamBody>,
) -> Response {
    let root = &state.team.run.team_root;
    let mut meta = match fs_store::load_team(root, &name) {
        Ok(meta) => meta,
        Err(_) => return error_404(&format!("team {name} not found")),
    };
    let captain = match state.store.get_node(&body.captain_node_id).await {
        Ok(Some(rec)) => rec,
        Ok(None) => {
            return error_400(format!(
                "captain node {} is not registered",
                body.captain_node_id
            ))
        }
        Err(e) => return error_500(format!("get_node: {e:#}")),
    };
    meta.captain = MemberRef {
        node_id: captain.id,
        name: captain.name,
    };
    save_team_respond(root, meta)
}

#[derive(Deserialize)]
pub struct MembersPatchBody {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// POST /api/teams/:name/members — add registered nodes (dedup, idempotent)
/// and/or remove members. The ONLY hard constraint: the captain can never be
/// removed (an empty member list is legal — captain-only teams can still
/// discuss).
pub async fn patch_members(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<MembersPatchBody>,
) -> Response {
    let root = &state.team.run.team_root;
    let mut meta = match fs_store::load_team(root, &name) {
        Ok(meta) => meta,
        Err(_) => return error_404(&format!("team {name} not found")),
    };
    let remove = dedup(body.remove);
    if remove.contains(&meta.captain.node_id) {
        return error_400("the captain cannot be removed from a team".to_string());
    }
    for id in dedup(body.add) {
        if meta.members.iter().any(|m| m.node_id == id) {
            continue; // idempotent add
        }
        match state.store.get_node(&id).await {
            Ok(Some(rec)) => meta.members.push(TeamMember {
                node_id: rec.id,
                name: rec.name,
                capabilities: Vec::new(),
                profiled_at: None,
            }),
            Ok(None) => return error_400(format!("member node {id} is not registered")),
            Err(e) => return error_500(format!("get_node: {e:#}")),
        }
    }
    meta.members.retain(|m| !remove.contains(&m.node_id));
    save_team_respond(root, meta)
}

fn save_team_respond(root: &std::path::Path, mut meta: TeamMeta) -> Response {
    meta.updated_at = now_ms();
    match fs_store::save_team(root, &meta) {
        Ok(()) => Json(json!({ "team": meta })).into_response(),
        Err(e) => error_500(format!("save_team: {e:#}")),
    }
}

/// POST /api/teams/:name/profile — 202: the interview runs in the background
/// (`profile_team` rewrites team.json when done; failures only warn).
pub async fn profile(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let root = state.team.run.team_root.clone();
    if fs_store::load_team(&root, &name).is_err() {
        return error_404(&format!("team {name} not found"));
    }
    let dispatcher = state.team.dispatcher.clone();
    let cfg = state.team.run.clone();
    tokio::spawn(async move {
        if let Err(error) = opencoder_team::profile_team(dispatcher, &cfg, &name).await {
            tracing::warn!(team = %name, error = %format!("{error:#}"), "profile_team failed");
        }
    });
    (StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response()
}

// ── helpers ───────────────────────────────────────────────────────────────

/// Order-preserving dedup (decide::dedup semantics, local for one list).
fn dedup(ids: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}
