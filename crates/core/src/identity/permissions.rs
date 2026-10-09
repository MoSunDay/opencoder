//! Shared HTTP authorization, before native routing or session forwarding.
use super::Role;

fn family(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.starts_with('/'))
}

pub fn read_operation(method: &str, path: &str) -> bool {
    matches!(method, "GET" | "HEAD")
        || method == "POST"
            && (path == "/api/brain/search"
                || path == "/api/todo/context-preview"
                || path == "/api/todo/validate-files"
                || path.starts_with("/api/ontology/") && path.ends_with("/vector-search"))
}

pub fn allowed(role: Role, method: &str, path: &str) -> bool {
    if role == Role::Admin {
        return true;
    }
    if path == "/" || path == "/favicon.ico" || path.starts_with("/static/") {
        return matches!(method, "GET" | "HEAD");
    }
    if [
        "/api/users",
        "/api/tokens",
        "/api/admin",
        "/api/config",
        "/api/metrics",
        "/metrics",
    ]
    .iter()
    .any(|root| family(path, root))
    {
        return false;
    }
    // Platform pages need node choices, not maintenance or machine channels.
    if family(path, "/api/nodes") {
        let parts: Vec<_> = path.trim_matches('/').split('/').collect();
        return matches!(method, "GET" | "HEAD")
            && (path == "/api/nodes"
                || parts.len() == 4 && matches!(parts[3], "dialogs" | "execution-capabilities"))
            || role == Role::Editor
                && parts.len() == 4
                && parts[3] == "dialogs"
                && method == "DELETE";
    }
    if matches!(
        path,
        "/api/me" | "/api/time" | "/api/health" | "/api/ready" | "/api/tui/agent-capabilities"
    ) {
        return matches!(method, "GET" | "HEAD");
    }
    // Resource transitions are issued by the execution runtime after cleanup.
    // Editors may cancel an execution, but cannot assert that its processes stopped.
    if path.starts_with("/api/executions/") && path.ends_with("/resources") {
        return matches!(method, "GET" | "HEAD");
    }
    let platform = [
        "/api/project",
        "/api/executions",
        "/api/sessions",
        "/api/agents",
        "/api/brain",
        "/api/dag",
        "/api/todo",
        "/api/teams",
        "/api/schedules",
        "/api/ontology",
        "/api/harnesses",
        "/api/models",
        "/api/skills",
    ]
    .iter()
    .any(|root| family(path, root));
    platform && (role == Role::Editor || read_operation(method, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roles_cover_platform_but_protect_administration() {
        for role in [Role::Viewer, Role::Editor] {
            for path in [
                "/api/users",
                "/api/tokens/x",
                "/api/admin/release",
                "/api/nodes/channel",
                "/api/nodes/n/maintenance",
            ] {
                assert!(!allowed(role, "GET", path), "{role:?} {path}");
                assert!(!allowed(role, "POST", path));
                assert!(allowed(Role::Admin, "POST", path));
            }
            assert!(allowed(role, "GET", "/api/nodes"));
            assert!(allowed(role, "GET", "/api/nodes/n/dialogs"));
        }
        for path in [
            "/api/project/todos/t",
            "/api/agents/a",
            "/api/ontology/envs/debug/entities",
            "/api/sessions/s/prompt",
            "/api/executions/x/commands",
            "/api/harnesses/codex",
        ] {
            assert!(allowed(Role::Viewer, "GET", path));
            for method in ["POST", "PUT", "PATCH", "DELETE"] {
                assert!(!allowed(Role::Viewer, method, path));
                assert!(allowed(Role::Editor, method, path));
            }
        }
        assert!(allowed(
            Role::Viewer,
            "POST",
            "/api/ontology/envs/debug/vector-search"
        ));
        assert!(!allowed(Role::Viewer, "POST", "/api/me"));
        assert!(!allowed(Role::Editor, "GET", "/api/unregistered"));
    }

    #[test]
    fn resource_ownership_writes_require_administration() {
        for role in [Role::Viewer, Role::Editor] {
            assert!(allowed(role, "GET", "/api/executions/run/resources"));
            assert!(!allowed(role, "POST", "/api/executions/run/resources"));
            assert!(!allowed(role, "GET", "/api/resource-admission-provider"));
            assert!(!allowed(role, "PUT", "/api/resource-admission-provider"));
        }
        assert!(allowed(
            Role::Admin,
            "POST",
            "/api/executions/run/resources"
        ));
        assert!(allowed(
            Role::Admin,
            "PUT",
            "/api/resource-admission-provider"
        ));
    }
}
