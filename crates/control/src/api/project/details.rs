//! Single-record reads used by the CLI and the project workbench.
use super::*;

pub async fn goal(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.projects.list_goals().await {
        Ok(rows) => match rows.into_iter().find(|row| row.id == id) {
            Some(row) => response(RpcReply::ok(json!(row))),
            None => response(RpcReply::error(404, "project not found")),
        },
        Err(error) => error_500(error.to_string()),
    }
}

pub async fn initiative(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.projects.list_initiatives(None).await {
        Ok(rows) => match rows.into_iter().find(|row| row.id == id) {
            Some(row) => response(RpcReply::ok(json!(row))),
            None => response(RpcReply::error(404, "initiative not found")),
        },
        Err(error) => error_500(error.to_string()),
    }
}

pub async fn todo(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.projects.get_todo(&id).await {
        Ok(Some(todo)) => match state.projects.list_todo_tags().await {
            Ok(tags) => {
                let mut body = json!(todo);
                body["tag_ids"] = json!(tags
                    .iter()
                    .filter(|tag| tag.todo_id == id)
                    .map(|tag| &tag.tag_id)
                    .collect::<Vec<_>>());
                response(RpcReply::ok(body))
            }
            Err(error) => error_500(error.to_string()),
        },
        Ok(None) => response(RpcReply::error(404, "todo not found")),
        Err(error) => error_500(error.to_string()),
    }
}
