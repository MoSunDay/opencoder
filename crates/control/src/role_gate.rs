//! Control-plane role gate: admins keep today's full surface; `user`/`root`
//! identities get a narrow read/launch profile (see [`allowed`]). The matrix
//! is deliberately a pure function so tests pin the exact wire contract.
//!
//! Auth-disabled deployments (no bearer middleware) never carry an
//! [`Identity`] extension and pass straight through — matching the
//! middleware's own pass-through semantics.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opencoder_core::identity::{Identity, Role};
use serde_json::json;

/// May this role touch this method+path? POST `/api/executions` and POST
/// `/api/executions/:id/commands` pass here for non-admins and get their
/// operator-only kind checks inside the executions handlers (the decision
/// needs the request body / execution index).
pub fn allowed(role: Role, method: &Method, path: &str) -> bool {
    match role {
        Role::Admin => true,
        Role::Root | Role::User => non_admin_allowed(method, path),
    }
}

fn non_admin_allowed(method: &Method, path: &str) -> bool {
    if crate::auth_mw::exempt(path) {
        return true;
    }
    if (path == "/api/me" || path == "/api/nodes") && method == Method::GET {
        return true;
    }
    if path == "/api/executions" {
        return method == Method::GET || method == Method::POST;
    }
    let Some(rest) = path.strip_prefix("/api/executions/") else {
        return false;
    };
    let Some((id, tail)) = rest.split_once('/') else {
        // `/api/executions/:id` — inspect.
        return method == Method::GET;
    };
    if id.is_empty() {
        return false;
    }
    match (tail, method) {
        // Commands on an execution: operator-only, enforced per-kind in the
        // handler once the index resolves the execution.
        ("commands", &Method::POST) => true,
        // Read-only subresources: events (SSE), pages, payloads, messages…
        (_, &Method::GET) => true,
        _ => false,
    }
}

pub async fn require_role(req: Request<Body>, next: Next) -> Response {
    let Some(identity) = req.extensions().get::<Identity>().cloned() else {
        return next.run(req).await;
    };
    if allowed(identity.role, req.method(), req.uri().path()) {
        next.run(req).await
    } else {
        denied(&identity)
    }
}

fn denied(identity: &Identity) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "ok": false,
            "error": format!("role '{}' may not access this endpoint", identity.role.as_str()),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(role: Role, method: &str, path: &str) -> bool {
        allowed(role, &Method::from_bytes(method.as_bytes()).unwrap(), path)
    }

    #[test]
    fn admin_keeps_the_full_surface() {
        for (method, path) in [
            ("GET", "/api/agents"),
            ("POST", "/api/executions"),
            ("PUT", "/api/harnesses/opencode"),
            ("DELETE", "/api/users/alice"),
        ] {
            assert!(allow(Role::Admin, method, path), "{method} {path}");
        }
    }

    #[test]
    fn non_admins_read_identity_nodes_and_executions() {
        for role in [Role::User, Role::Root] {
            assert!(allow(role, "GET", "/api/me"));
            assert!(allow(role, "GET", "/api/nodes"));
            assert!(allow(role, "GET", "/api/executions"));
            assert!(allow(role, "GET", "/api/executions/agent-x"));
            assert!(allow(role, "GET", "/api/executions/agent-x/events"));
            assert!(allow(role, "GET", "/api/executions/agent-x/events/3/payload"));
            assert!(allow(role, "GET", "/api/executions/agent-x/messages"));
            assert!(allow(role, "GET", "/static/app.js"));
            // Writes outside the execution surface stay admin-only.
            assert!(!allow(role, "GET", "/api/agents"));
            assert!(!allow(role, "GET", "/api/sessions/agent-x/events"));
            assert!(!allow(role, "PUT", "/api/nodes/n1/scheduling"));
            assert!(!allow(role, "POST", "/api/nodes/n1/maintenance"));
            assert!(!allow(role, "GET", "/api/users"));
            assert!(!allow(role, "POST", "/api/users"));
            assert!(!allow(role, "DELETE", "/api/users/alice"));
            assert!(!allow(role, "GET", "/api/brain/capabilities"));
            // The shell/assets stay reachable (auth exempts them).
            assert!(allow(role, "GET", "/"));
        }
    }

    #[test]
    fn non_admin_submissions_and_commands_pass_to_kind_checks() {
        // These pass the gate; the executions handlers reject non-operator
        // kinds for non-admins.
        assert!(allow(Role::User, "POST", "/api/executions"));
        assert!(allow(Role::User, "POST", "/api/executions/operator-x/commands"));
        // Other mutations on executions stay closed.
        assert!(!allow(Role::User, "PUT", "/api/executions/operator-x"));
        assert!(!allow(Role::User, "POST", "/api/executions/operator-x/events"));
        assert!(!allow(Role::User, "POST", "/api/executions/"));
    }
}
