use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("dependency unavailable: {0}")]
    Dependency(String),
    #[error("database operation failed: {0}")]
    Database(#[from] libsql::Error),
    #[error("file operation failed")]
    Io(#[from] std::io::Error),
}

impl AppError {
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
    pub fn dependency(message: impl Into<String>) -> Self {
        Self::Dependency(message.into())
    }
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Invalid(message) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", message)
            }
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "resource not found".into(),
            ),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "authentication required".into(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "operation is not allowed".into(),
            ),
            Self::Dependency(message) => {
                (StatusCode::BAD_GATEWAY, "dependency_unavailable", message)
            }
            Self::Config(_) | Self::Database(_) | Self::Io(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal service error".into(),
            ),
        };
        (
            status,
            Json(ErrorBody {
                ok: false,
                error: message.clone(),
                code,
                message,
            }),
        )
            .into_response()
    }
}
