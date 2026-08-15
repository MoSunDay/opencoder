use opencoder_core::InjectionTarget;
use serde_json::{json, Value};

/// Build a save/upsert merge-patch for one MCP server.
pub fn save_mcp_json(
    name: &str,
    enabled: bool,
    inject_to: InjectionTarget,
    command: Option<&str>,
    args: &[String],
    url: Option<&str>,
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
    json!({ "mcp_servers": { name: Value::Object(server) } })
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
        );
        assert_eq!(v["mcp_servers"]["myserver"]["enabled"], true);
        assert_eq!(v["mcp_servers"]["myserver"]["command"], "npx");
        assert_eq!(v["mcp_servers"]["myserver"]["args"][0], "-y");
        assert_eq!(v["mcp_servers"]["myserver"]["args"][1], "@mcp/srv");
        assert_eq!(v["mcp_servers"]["myserver"]["url"], "http://x");
    }

    #[test]
    fn save_omits_empty_optional_fields() {
        let v = save_mcp_json("bare", false, InjectionTarget::parent_only(), None, &[], None);
        assert_eq!(v["mcp_servers"]["bare"]["enabled"], false);
        assert!(v["mcp_servers"]["bare"].get("command").is_none());
        assert!(v["mcp_servers"]["bare"].get("args").is_none());
        assert!(v["mcp_servers"]["bare"].get("url").is_none());
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
