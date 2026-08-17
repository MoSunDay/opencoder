use opencoder_core::InjectionTarget;
use serde_json::{json, Value};

/// Build a save/upsert merge-patch for one MCP server.
///
/// `renamed_from` carries the server's pre-edit key (edit mode only). When it
/// differs from `name`, the domain object also sets the old key to null —
/// merge-patch semantics delete nulled keys, so without this a rename would
/// leave both `old` and `name` in mcp.json and both servers would connect.
/// The `old == name` filter lives here (an unconditional null would
/// self-delete the just-saved server), so callers may pass `original_name`
/// as-is.
pub fn save_mcp_json(
    name: &str,
    enabled: bool,
    inject_to: InjectionTarget,
    command: Option<&str>,
    args: &[String],
    url: Option<&str>,
    renamed_from: Option<&str>,
) -> Value {
    let mut server = serde_json::Map::new();
    server.insert("enabled".to_string(), json!(enabled));
    server.insert("inject_to".to_string(), json!(inject_to));
    if let Some(c) = command {
        server.insert("command".to_string(), json!(c));
    }
    if !args.is_empty() {
        server.insert(
            "args".to_string(),
            json!(args.iter().map(String::as_str).collect::<Vec<_>>()),
        );
    }
    if let Some(u) = url {
        server.insert("url".to_string(), json!(u));
    }
    // Built via an explicit Map so the old (null) and new keys provably
    // coexist in one object — a nested `json!` with a variable key makes
    // that invariant too easy to break silently.
    let mut servers = serde_json::Map::new();
    if let Some(old) = renamed_from.filter(|old| *old != name) {
        servers.insert(old.to_string(), Value::Null);
    }
    servers.insert(name.to_string(), Value::Object(server));
    json!({ "mcp_servers": Value::Object(servers) })
}

/// Toggle a server's `enabled` flag.
pub fn toggle_mcp_json(name: &str, enabled: bool) -> Value {
    json!({ "mcp_servers": { name: { "enabled": enabled } } })
}

/// Delete a server (null = remove key in merge-patch).
pub fn delete_mcp_json(name: &str) -> Value {
    json!({ "mcp_servers": { name: Value::Null } })
}

/// Normalize an MCP server name the same way the session runtime does when
/// it builds tool names (`mcp__{normalized}__{tool}`): `-` and `.` both
/// become `_`, so `a-b` / `a.b` / `a_b` all share the `mcp__a_b__…` prefix.
/// Mirrors `McpTool::sanitize_server_name` in
/// `crates/session/src/mcp/tool.rs` — deliberately duplicated (a one-liner
/// both sides, cross-referenced by comments) instead of adding a cross-crate
/// dependency; both copies carry pinning tests.
pub fn normalized_server_name(name: &str) -> String {
    name.replace(['-', '.'], "_")
}

/// Would saving server `new` collide — after normalization — with a
/// differently-named server already present in `existing`? Returns the
/// conflicting existing name when so.
///
/// Exclusion rules (bug #14: a clash would silently shadow the other
/// server's tools and bypass its `inject_to` scope at registration time):
/// - `renamed_from` is the pre-edit key vacated by the *same* patch (the
///   null delete marker of a rename); it gives way and never counts.
/// - An entry whose original name equals `new` is an update-in-place, not a
///   collision — callers pass the already-configured key set as `existing`,
///   so the server being re-saved is in there under its own name.
pub fn colliding_server(
    new: &str,
    renamed_from: Option<&str>,
    existing: &[String],
) -> Option<String> {
    let normalized = normalized_server_name(new);
    existing
        .iter()
        .filter(|name| Some(name.as_str()) != renamed_from && name.as_str() != new)
        .find(|name| normalized_server_name(name) == normalized)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_includes_all_fields() {
        let args = vec!["-y".to_string(), "@mcp/srv".to_string()];
        let v = save_mcp_json(
            "myserver",
            true,
            InjectionTarget::parent_only(),
            Some("npx"),
            &args,
            Some("http://x"),
            None,
        );
        assert_eq!(v["mcp_servers"]["myserver"]["enabled"], true);
        assert_eq!(v["mcp_servers"]["myserver"]["command"], "npx");
        assert_eq!(v["mcp_servers"]["myserver"]["args"][0], "-y");
        assert_eq!(v["mcp_servers"]["myserver"]["args"][1], "@mcp/srv");
        assert_eq!(v["mcp_servers"]["myserver"]["url"], "http://x");
    }

    #[test]
    fn save_omits_empty_optional_fields() {
        let v = save_mcp_json(
            "bare",
            false,
            InjectionTarget::parent_only(),
            None,
            &[],
            None,
            None,
        );
        assert_eq!(v["mcp_servers"]["bare"]["enabled"], false);
        assert!(v["mcp_servers"]["bare"].get("command").is_none());
        assert!(v["mcp_servers"]["bare"].get("args").is_none());
        assert!(v["mcp_servers"]["bare"].get("url").is_none());
    }

    #[test]
    fn save_nulls_old_key_on_rename() {
        let args = vec!["-y".to_string()];
        let v = save_mcp_json(
            "b",
            true,
            InjectionTarget::parent_only(),
            Some("npx"),
            &args,
            None,
            Some("a"),
        );
        assert!(v["mcp_servers"]["a"].is_null(), "old key must be nulled");
        assert!(
            v["mcp_servers"]["b"].is_object(),
            "new key must carry the entry"
        );
        assert_eq!(v["mcp_servers"]["b"]["enabled"], true);
        assert_eq!(v["mcp_servers"]["b"]["command"], "npx");
    }

    #[test]
    fn save_keeps_server_when_name_unchanged() {
        let v = save_mcp_json(
            "a",
            true,
            InjectionTarget::parent_only(),
            Some("npx"),
            &[],
            None,
            Some("a"),
        );
        assert!(
            v["mcp_servers"]["a"].is_object(),
            "unchanged name must not self-delete"
        );
    }

    /// RFC 7396 mirror of `opencoder_core`'s config merge (it is `pub(super)`
    /// there, hence not importable from this crate): null deletes a key,
    /// objects merge recursively. Used to prove the rename patch semantics.
    fn apply_merge_patch(dst: &mut Value, patch: &Value) {
        match (dst, patch) {
            (Value::Object(d), Value::Object(p)) => {
                for (k, pv) in p {
                    match (d.get_mut(k), pv) {
                        (Some(Value::Object(_)), Value::Object(_)) => {
                            if let Some(child) = d.get_mut(k) {
                                apply_merge_patch(child, pv);
                            }
                        }
                        (_, Value::Null) => {
                            d.remove(k);
                        }
                        _ => {
                            d.insert(k.clone(), pv.clone());
                        }
                    }
                }
            }
            (d, p) => *d = p.clone(),
        }
    }

    #[test]
    fn rename_patch_removes_old_key_after_merge() {
        let mut config = json!({"mcp_servers": {"a": {"enabled": true, "command": "old"}}});
        let patch = save_mcp_json(
            "b",
            true,
            InjectionTarget::parent_only(),
            Some("npx"),
            &[],
            None,
            Some("a"),
        );
        apply_merge_patch(&mut config, &patch);
        assert!(
            config["mcp_servers"].get("a").is_none(),
            "merged config must drop the old key"
        );
        assert_eq!(config["mcp_servers"]["b"]["command"], "npx");
        assert_eq!(
            config["mcp_servers"].as_object().map(|o| o.len()),
            Some(1),
            "exactly one server after rename"
        );
    }

    #[test]
    fn toggle_sets_enabled_flag() {
        let v = toggle_mcp_json("srv", true);
        assert_eq!(v["mcp_servers"]["srv"]["enabled"], true);
        let v2 = toggle_mcp_json("srv", false);
        assert_eq!(v2["mcp_servers"]["srv"]["enabled"], false);
    }

    #[test]
    fn delete_emits_null_for_key_removal() {
        let v = delete_mcp_json("old");
        assert!(v["mcp_servers"]["old"].is_null());
    }

    #[test]
    fn normalized_server_name_is_table_driven() {
        for (raw, norm) in [
            ("a-b", "a_b"),
            ("a.b", "a_b"),
            ("a_b", "a_b"),
            ("plain", "plain"),
            ("x-y.z_w", "x_y_z_w"),
            ("A-B.C", "A_B_C"),
            ("", ""),
        ] {
            assert_eq!(normalized_server_name(raw), norm, "raw = {raw:?}");
        }
    }

    #[test]
    fn colliding_server_detects_normalized_twin() {
        // `a_b` vs existing `a-b`: same normalized prefix, distinct names.
        assert_eq!(
            colliding_server("a_b", None, &["a-b".to_string()]),
            Some("a-b".to_string())
        );
    }

    #[test]
    fn colliding_server_ignores_vacated_rename_key() {
        // Rename `a.b` → `a-b`: the old key is nulled by the same patch, so
        // it gives way and the rename lands on its own normalized slot.
        assert_eq!(
            colliding_server("a-b", Some("a.b"), &["a-b".to_string(), "a.b".to_string()],),
            None
        );
    }

    #[test]
    fn colliding_server_ignores_disjoint_names() {
        assert_eq!(colliding_server("x", None, &["y".to_string()]), None);
    }

    #[test]
    fn colliding_server_ignores_same_original_name() {
        // Re-saving `a-b` while `a-b` is already configured: existing always
        // contains the server's own key, which must not self-collide.
        assert_eq!(colliding_server("a-b", None, &["a-b".to_string()]), None);
    }
}
