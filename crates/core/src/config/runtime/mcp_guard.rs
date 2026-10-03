//! Save-time MCP server name-collision guard (bug #14).
//!
//! Two configured server names that normalize to the same tool prefix
//! (`a-b` / `a.b` / `a_b` all become `a_b`) would register under one
//! `mcp__a_b__*` namespace and silently shadow each other's tools (plus
//! bypass `inject_to` scoping). The TUI form refuses such a save up front
//! (`crates/tui/src/mcp_menu/patch.rs::colliding_server`); this module is
//! the crate-level second net wired into the config save path, so every
//! writer (web `PATCH /api/config` included) is covered.

use std::path::Path;

use serde_json::Value;

/// Normalize a server name the way tool registration does: `-` and `.` both
/// become `_` (it builds tool names `mcp__{normalized}__{tool}`).
///
/// Deliberately duplicated one-liner (existing convention): mirrors
/// `crates/session/src/mcp/tool.rs::sanitize_server_name` (registration
/// side) and `crates/tui/src/mcp_menu/patch.rs::normalized_server_name`
/// (form pre-check side) — cross-referenced here, each copy pinned by its
/// own table-driven test instead of a cross-crate dep.
pub fn normalized_server_name(name: &str) -> String {
    name.replace(['-', '.'], "_")
}

/// Pairwise collision detection over the **non-null** entries of a merged
/// `mcp_servers` map: returns `(offending, existing)` when two distinct
/// keys normalize to the same tool prefix, in map iteration order (the
/// pair is unordered for messaging purposes — both names are reported).
///
/// `null` entries are merge-patch delete markers and never collide. Because
/// the check runs on the **merged** map at save time, the TUI pre-check's
/// exemptions hold here for free:
/// - a rename's old key (`a: null` in the same patch) is already removed by
///   the merge, so `renamed_from` needs no explicit exemption;
/// - re-saving a server under its own key is an update-in-place — the
///   merged map still holds exactly one entry, which cannot self-collide.
pub fn mcp_name_collision(servers: &serde_json::Map<String, Value>) -> Option<(String, String)> {
    let normalized: Vec<(&String, String)> = servers
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, _)| (k, normalized_server_name(k)))
        .collect();
    (0..normalized.len()).find_map(|i| {
        normalized[i + 1..]
            .iter()
            .find(|(_, n)| *n == normalized[i].1)
            .map(|(k, _)| ((*k).clone(), normalized[i].0.clone()))
    })
}

/// Human-facing error text for a [`mcp_name_collision`] hit: names both
/// raw keys plus the shared normalized tool prefix they would fight over.
pub fn conflict_message(offending: &str, existing: &str) -> String {
    format!(
        "mcp server name conflict: \"{offending}\" collides with existing \
         \"{existing}\" (both normalize to mcp__{}__; their tools would \
         shadow each other — rename one of the servers)",
        normalized_server_name(offending)
    )
}

/// Dry-run the save-time guard for a `Config::save(working_dir, patch)`
/// without touching disk: returns the [`conflict_message`] when the merge
/// the save would perform lands on a name collision, else `None`. Mirrors
/// the domain routing (`mcp_servers` always lands in mcp.json, never in
/// config.json), so it needs no config.json fallback probe.
///
/// Read failures yield `None` (no conflict) — the subsequent save surfaces
/// them as errors. The check-then-save window is unchecked: the save-time
/// guard is the authoritative net.
pub fn mcp_name_conflict_in_patch(working_dir: &Path, patch: &Value) -> Option<String> {
    super::domain::probe_mcp_conflict(working_dir, patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn servers(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn normalized_server_name_is_table_driven() {
        // Pinning twin of the tables in session's `sanitize_server_name`
        // and the TUI's `normalized_server_name` — all three must agree.
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
    fn collision_detects_normalized_twins() {
        let hit = mcp_name_collision(&servers(json!({
            "a-b": { "command": "x" },
            "a.b": { "url": "http://y" },
        })));
        let (x, y) = hit.expect("normalized twins must collide");
        assert!(
            (x == "a.b" && y == "a-b") || (x == "a-b" && y == "a.b"),
            "pair must be exactly the two raw names: ({x}, {y})"
        );
    }

    #[test]
    fn collision_ignores_null_delete_markers() {
        // Rename shape: the vacated key is a null marker, not a server.
        assert_eq!(
            mcp_name_collision(&servers(json!({
                "a.b": { "command": "x" },
                "a-b": null,
            }))),
            None
        );
        assert_eq!(mcp_name_collision(&servers(json!({ "a-b": null }))), None);
    }

    #[test]
    fn collision_ignores_disjoint_and_single_entries() {
        assert_eq!(
            mcp_name_collision(&servers(json!({
                "a-b": { "command": "x" },
                "c.d": { "command": "y" },
            }))),
            None
        );
        assert_eq!(
            mcp_name_collision(&servers(json!({ "only": { "command": "x" } }))),
            None
        );
        assert_eq!(mcp_name_collision(&serde_json::Map::new()), None);
    }

    #[test]
    fn collision_catches_three_way_normalized_clash() {
        let hit = mcp_name_collision(&servers(json!({
            "a_b": { "command": "x" },
            "a-b": { "command": "y" },
            "a.b": { "command": "z" },
        })));
        assert!(hit.is_some(), "any two of three normalized twins collide");
    }

    #[test]
    fn conflict_message_names_both_and_normalized_prefix() {
        let msg = conflict_message("a.b", "a-b");
        assert!(msg.contains("\"a.b\"") && msg.contains("\"a-b\""), "{msg}");
        assert!(msg.contains("mcp__a_b__"), "{msg}");
    }
}
