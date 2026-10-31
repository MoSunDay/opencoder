//! HTTP surface for freshness policies, runs and reviews.
use crate::{
    api::{resolve_env, ListQuery},
    auth::Actor,
    database::FreshnessPolicyFields,
    domain::{FreshnessExecutor, FreshnessVerdict},
    error::AppError,
    http::AppState,
};
use axum::{
    extract::{Path, Query, State},
    routing::{get, patch},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/envs/:env/freshness/policies",
            get(policies).post(create_policy),
        )
        .route("/envs/:env/freshness/policies/:id", patch(update_policy).get(policy))
        .route("/envs/:env/freshness/runs", get(runs))
        .route("/envs/:env/freshness/runs/:id", get(run))
        .route("/envs/:env/freshness/reviews", get(reviews).post(create_review))
}

async fn policies(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        json!({"items":state.database.store().await.freshness_policies(env.env_num, query.include_deleted).await?}),
    ))
}

async fn policy(
    State(state): State<AppState>,
    Path((env, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        json!({"item":state.database.store().await.freshness_policy(env.env_num, id).await?}),
    ))
}

#[derive(Deserialize)]
struct PolicyBody {
    name: String,
    #[serde(default)]
    description: String,
    entity_type_id: Uuid,
    executor: FreshnessExecutor,
    concurrency: u32,
    writeback_attribute_key: Option<String>,
}

impl PolicyBody {
    fn fields(&self) -> FreshnessPolicyFields<'_> {
        FreshnessPolicyFields {
            name: &self.name,
            description: &self.description,
            entity_type_id: self.entity_type_id,
            executor: self.executor.clone(),
            concurrency: self.concurrency,
            writeback_attribute_key: self.writeback_attribute_key.as_deref(),
        }
    }
}

async fn create_policy(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<PolicyBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let item = state
        .database
        .store()
        .await
        .create_freshness_policy(env.env_num, body.fields(), &actor.external_id)
        .await?;
    Ok(Json(json!({"item":item})))
}

#[derive(Deserialize)]
struct PolicyUpdateBody {
    #[serde(flatten)]
    policy: PolicyBody,
    is_deleted: bool,
    expected_revision: i64,
}

async fn update_policy(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((env, id)): Path<(String, Uuid)>,
    Json(body): Json<PolicyUpdateBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let store = state.database.store().await;
    let item = store
        .update_freshness_policy(
            env.env_num,
            id,
            body.policy.fields(),
            body.is_deleted,
            body.expected_revision,
            &actor.external_id,
        )
        .await?;
    Ok(Json(json!({"revision":item.revision})))
}

#[derive(Deserialize)]
struct RunListQuery {
    #[serde(default)]
    include_deleted: bool,
    #[serde(default)]
    offset: u32,
    limit: Option<u32>,
    policy_id: Option<Uuid>,
}

async fn runs(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<RunListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    let limit = query.limit.unwrap_or(50).min(200);
    let items = state
        .database
        .store()
        .await
        .freshness_runs(env.env_num, query.include_deleted, query.offset, limit)
        .await?
        .into_iter()
        .filter(|run| query.policy_id.is_none_or(|id| run.policy_id == id))
        .collect::<Vec<_>>();
    Ok(Json(json!({"items":items})))
}

async fn run(
    State(state): State<AppState>,
    Path((env, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        json!({"item":state.database.store().await.freshness_run(env.env_num, id).await?}),
    ))
}

#[derive(Deserialize)]
struct ReviewListQuery {
    run_id: Option<Uuid>,
}

async fn reviews(
    State(state): State<AppState>,
    Path(env): Path<String>,
    Query(query): Query<ReviewListQuery>,
) -> Result<Json<Value>, AppError> {
    let env = resolve_env(&state, &env).await?;
    Ok(Json(
        json!({"items":state.database.store().await.freshness_reviews(env.env_num, query.run_id).await?}),
    ))
}

#[derive(Deserialize)]
struct ReviewBody {
    run_id: Uuid,
    entity_id: Uuid,
    verdict: FreshnessVerdict,
    #[serde(default)]
    comment: String,
}

async fn create_review(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(env): Path<String>,
    Json(body): Json<ReviewBody>,
) -> Result<Json<Value>, AppError> {
    actor.require_manage()?;
    let env = resolve_env(&state, &env).await?;
    let result = state
        .database
        .store()
        .await
        .create_freshness_review(
            env.env_num,
            body.run_id,
            body.entity_id,
            body.verdict,
            &body.comment,
            &state.text_store,
            &actor.external_id,
        )
        .await?;
    Ok(Json(result))
}
