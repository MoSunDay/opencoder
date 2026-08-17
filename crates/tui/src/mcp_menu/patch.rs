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
}
