pub mod admission;
pub mod brain;
pub(crate) mod brain_dispatch;
pub mod catalog;
pub mod executions;
pub mod project;
pub mod project_util;
pub mod session;
pub mod settings;
pub mod users;
pub mod stream;
pub mod streaming;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::fleet::RpcReply;
pub fn response(reply: RpcReply) -> Response {
    (
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(reply.body),
    )
        .into_response()
}
pub fn error_400(msg: String) -> Response {
    response(RpcReply::error(400, msg))
}
pub fn error_404(msg: &str) -> Response {
    response(RpcReply::error(404, msg))
}
pub fn error_409(msg: &str) -> Response {
    response(RpcReply::error(409, msg))
}
pub fn error_500(msg: String) -> Response {
    response(RpcReply::error(500, msg))
}

pub mod compat;

mod template;
