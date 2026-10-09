//! Project and initiative tag catalog; same handlers on both HTTP surfaces.
use crate::{
    api_project_util::{error_400, error_404, error_409, error_500, require_deps},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use opencoder_store::project::{tags::TagError, ProjectTag};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct TagQuery {
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
}
#[derive(Deserialize)]
pub struct TagBody {
    pub scope_type: String,
    pub scope_id: String,
    pub name: String,
}

pub fn tag_error(error: anyhow::Error) -> Response {
    match error.downcast_ref::<TagError>() {
        Some(TagError::Duplicate) => error_409(error.to_string()),
        Some(TagError::InvalidScope) => error_400(error.to_string()),
        None => error_500(format!("project tags: {error:#}")),
    }
}

pub async fn list(State(state): State<Arc<AppState>>, Query(query): Query<TagQuery>) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.list_tags().await {
        Ok(tags) => Json(json!({"tags":tags.into_iter().filter(|tag| query.scope_type.as_ref().is_none_or(|scope| &tag.scope_type == scope) && query.scope_id.as_ref().is_none_or(|id| &tag.scope_id == id)).collect::<Vec<_>>()})).into_response(),
        Err(error) => tag_error(error),
    }
}

async fn validate_body(
    projects: &dyn opencoder_store::ProjectStore,
    body: &TagBody,
) -> Result<(), Box<Response>> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 128 {
        return Err(Box::new(error_400(
            "tag name must contain 1–128 characters",
        )));
    }
    let exists = match body.scope_type.as_str() {
        "project" => projects
            .list_goals()
            .await
            .map(|rows| rows.iter().any(|r| r.id == body.scope_id)),
        "initiative" => projects
            .list_initiatives(None)
            .await
            .map(|rows| rows.iter().any(|r| r.id == body.scope_id)),
        _ => {
            return Err(Box::new(error_400(
                "tag scope must be project or initiative",
            )))
        }
    };
    match exists {
        Ok(true) => Ok(()),
        Ok(false) => Err(Box::new(error_404("tag scope not found"))),
        Err(error) => Err(Box::new(tag_error(error))),
    }
}

pub async fn create(State(state): State<Arc<AppState>>, Json(body): Json<TagBody>) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    if let Err(response) = validate_body(deps.projects.as_ref(), &body).await {
        return *response;
    }
    let tag = ProjectTag {
        id: format!("tag-{}", ulid::Ulid::new()),
        scope_type: body.scope_type,
        scope_id: body.scope_id,
        name: body.name.trim().into(),
    };
    match deps.projects.write_tag(&tag).await {
        Ok(()) => Json(json!(tag)).into_response(),
        Err(error) => tag_error(error),
    }
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<TagBody>,
) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let mut tag = match deps.projects.list_tags().await {
        Ok(tags) => match tags.into_iter().find(|tag| tag.id == id) {
            Some(tag) => tag,
            None => return error_404("tag not found"),
        },
        Err(error) => return tag_error(error),
    };
    if let Err(response) = validate_body(deps.projects.as_ref(), &body).await {
        return *response;
    }
    tag.name = body.name.trim().into();
    tag.scope_type = body.scope_type;
    tag.scope_id = body.scope_id;
    match deps.projects.write_tag(&tag).await {
        Ok(()) => Json(json!(tag)).into_response(),
        Err(error) => tag_error(error),
    }
}

pub async fn delete(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let deps = match require_deps(&state) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    match deps.projects.delete_tag(&id).await {
        Ok(true) => Json(json!({"deleted":true})).into_response(),
        Ok(false) => error_404("tag not found"),
        Err(error) => tag_error(error),
    }
}
