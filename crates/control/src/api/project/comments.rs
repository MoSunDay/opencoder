//! Explicit TODO notes authored by the authenticated user or a named capability.
use super::*;
use axum::Extension;
use opencoder_core::identity::Identity;
use opencoder_store::project::comments::{CommentError, CommentMutation, CommentQuery, NewComment};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentBody {
    pub request_id: String,
    pub body_md: String,
    pub capability_id: Option<String>,
    pub execution_id: Option<String>,
}

fn failure(error: anyhow::Error) -> Response {
    let status = match error.downcast_ref::<CommentError>() {
        Some(CommentError::MissingTodo | CommentError::MissingComment) => 404,
        Some(CommentError::Forbidden) => 403,
        Some(CommentError::InvalidInput) => 400,
        Some(
            CommentError::Conflict
            | CommentError::InvalidSource
            | CommentError::Deleted
            | CommentError::RevisionConflict,
        ) => 409,
        None => 500,
    };
    response(RpcReply::error(status, error.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBody {
    pub body_md: String,
    pub expected_revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteQuery {
    pub expected_revision: i64,
}

pub async fn edit(
    State(state): State<Arc<AppState>>,
    Path((id, seq)): Path<(String, i64)>,
    identity: Option<Extension<Identity>>,
    Json(body): Json<EditBody>,
) -> Response {
    mutate(
        state,
        id,
        seq,
        identity,
        body.expected_revision,
        Some(body.body_md),
    )
    .await
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path((id, seq)): Path<(String, i64)>,
    identity: Option<Extension<Identity>>,
    Query(query): Query<DeleteQuery>,
) -> Response {
    mutate(state, id, seq, identity, query.expected_revision, None).await
}

async fn mutate(
    state: Arc<AppState>,
    id: String,
    seq: i64,
    identity: Option<Extension<Identity>>,
    expected_revision: i64,
    body_md: Option<String>,
) -> Response {
    let actor = identity
        .map(|Extension(value)| value)
        .unwrap_or_else(|| Identity::admin("admin"));
    if !actor.can_edit() {
        return response(RpcReply::error(403, "当前角色无权修改评论"));
    }
    let change = CommentMutation {
        todo_id: id,
        seq,
        expected_revision,
        body_md,
        actor,
        now: opencoder_core::message::now_ms(),
    };
    match state.projects.mutate_todo_comment(&change).await {
        Ok(Some(comment)) => response(RpcReply::ok(json!(comment))),
        Ok(None) => response(RpcReply::ok(json!({"ok":true}))),
        Err(error) => failure(error),
    }
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<CommentQuery>,
) -> Response {
    if !query.valid() {
        return error_400("评论分页参数无效".into());
    }
    match state.projects.todo_comments(&id, query).await {
        Ok(Some(page)) => response(RpcReply::ok(json!(page))),
        Ok(None) => response(RpcReply::error(404, "TODO 不存在")),
        Err(error) => failure(error),
    }
}

pub async fn append(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    identity: Option<Extension<Identity>>,
    Json(body): Json<CommentBody>,
) -> Response {
    let identity = identity
        .map(|Extension(value)| value)
        .unwrap_or_else(|| Identity::admin("admin"));
    if !identity.can_edit() {
        return response(RpcReply::error(403, "当前角色无权发表评论"));
    }
    if body.request_id.is_empty()
        || body.request_id.len() > 128
        || !body
            .request_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        || body.body_md.trim().is_empty()
        || body.body_md.len() > 65536
        || [&body.capability_id, &body.execution_id].iter().any(|id| {
            id.as_ref()
                .is_some_and(|id| id.trim().is_empty() || id.len() > 255)
        })
    {
        return error_400("评论不能为空且须在 64 KiB 以内；request_id 和来源 ID 必须有效".into());
    }
    match state
        .projects
        .todo_comment_request(&id, &identity.name, &body.request_id)
        .await
    {
        Ok(Some(old)) => {
            return if old.matches_input(
                &body.body_md,
                body.capability_id.as_deref(),
                body.execution_id.as_deref(),
            ) {
                response(RpcReply::ok(json!(old)))
            } else {
                response(RpcReply::error(409, "相同 request_id 已用于其他评论内容"))
            }
        }
        Err(error) => return failure(error),
        Ok(None) => {}
    }
    let (author_kind, author_id, author_name, capability_kind) =
        if let Some(capability) = &body.capability_id {
            let catalog = match super::super::brain_runs::catalog::capabilities(&state).await {
                Ok(catalog) => catalog,
                Err(error) => return failure(error),
            };
            let Some(cap) = catalog.iter().find(|cap| {
                cap["id"] == *capability
                    && matches!(
                        cap["kind"].as_str(),
                        Some("agent" | "operator" | "team" | "dag" | "todos" | "brain")
                    )
            }) else {
                return response(RpcReply::error(404, "评论署名能力不存在"));
            };
            (
                "capability",
                capability.clone(),
                cap["target"].as_str().unwrap_or(capability).to_owned(),
                cap["kind"].as_str().map(str::to_owned),
            )
        } else {
            ("user", identity.name.clone(), identity.name.clone(), None)
        };
    if let Some(execution) = &body.execution_id {
        let links = match state.projects.list_todo_assignments(&id).await {
            Ok(links) => links,
            Err(error) => return failure(error),
        };
        if !links.iter().any(|link| &link.execution_id == execution) {
            return response(RpcReply::error(409, "评论来源必须已关联当前 TODO"));
        }
        if let Some(capability) = &body.capability_id {
            let index = match state.fleet.index(execution).await {
                Ok(Some(index)) => index,
                Ok(None) => return response(RpcReply::error(404, "执行来源不存在")),
                Err(error) => return failure(error),
            };
            if let Err(reply) =
                super::super::project_links::reference::resolve(&state, &index, Some(capability))
                    .await
            {
                return response(reply);
            }
        }
    }
    let entry = NewComment {
        todo_id: id,
        request_id: body.request_id,
        body_md: body.body_md,
        author_kind: author_kind.into(),
        author_id,
        author_name,
        capability_kind,
        submitted_by: identity.name,
        execution_id: body.execution_id,
        created_at: opencoder_core::message::now_ms(),
    };
    match state.projects.append_todo_comment(&entry).await {
        Ok(comment) => response(RpcReply::ok(json!(comment))),
        Err(error) => failure(error),
    }
}
