use crate::error::AppError;
use axum::{extract::Request, middleware::Next, response::Response};
use opencoder_core::identity::Identity;

#[derive(Clone)]
pub(crate) struct Actor {
    pub external_id: String,
    pub display_name: String,
    pub can_manage: bool,
    pub is_service: bool,
}

impl Actor {
    pub fn require_manage(&self) -> Result<(), AppError> {
        if self.can_manage {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
}

pub(crate) async fn identify(mut request: Request, next: Next) -> Response {
    // The enclosing OpenCoder router owns credential validation. Its auth-disabled
    // mode is intentionally preserved for local deployments and isolated tests.
    let identity = request
        .extensions()
        .get::<Identity>()
        .cloned()
        .unwrap_or_else(|| Identity::admin("local"));
    request.extensions_mut().insert(Actor {
        external_id: identity.name.clone(),
        display_name: identity.name.clone(),
        can_manage: identity.can_edit(),
        is_service: false,
    });
    next.run(request).await
}
