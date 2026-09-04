//! Project-data storage backend selection.
//!
//! The project module (goals / milestones / todos / runs) defaults to the
//! embedded libsql store; `mysql` / `starrocks` can serve the project tables
//! from an external DB instead. DSN values may reference environment
//! variables via `{VAR}` so credentials never land in the config file.

use serde::{Deserialize, Serialize};

/// Which backend serves the project tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    /// Embedded libsql (local SQLite, WAL) — always available, the default.
    #[default]
    Libsql,
    /// External MySQL serving the project tables only.
    Mysql,
    /// External StarRocks serving the project tables only.
    Starrocks,
}

impl StorageBackend {
    /// Wire name, identical to the serde form.
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageBackend::Libsql => "libsql",
            StorageBackend::Mysql => "mysql",
            StorageBackend::Starrocks => "starrocks",
        }
    }

    /// Case-insensitive parse; `None` for unknown backends (callers surface
    /// the typo instead of silently falling back to libsql).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "libsql" => Some(StorageBackend::Libsql),
            "mysql" => Some(StorageBackend::Mysql),
            "starrocks" => Some(StorageBackend::Starrocks),
            _ => None,
        }
    }
}

/// Project-storage configuration; `{}` deserializes to the libsql default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Active backend for the project tables.
    #[serde(default)]
    pub backend: StorageBackend,
    /// MySQL DSN, used only when `backend == mysql`. May contain `{VAR}` refs
    /// expanded at read time by [`expand_env_vars`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mysql: Option<String>,
    /// StarRocks DSN, used only when `backend == starrocks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starrocks: Option<String>,
}

impl StorageConfig {
    /// DSN for the configured backend (`None` for libsql — embedded, no DSN),
    /// with `{VAR}` references expanded from the environment.
    pub fn dsn(&self) -> Option<String> {
        let raw = match self.backend {
            StorageBackend::Libsql => return None,
            StorageBackend::Mysql => self.mysql.as_deref()?,
            StorageBackend::Starrocks => self.starrocks.as_deref()?,
        };
        Some(expand_env_vars(raw))
    }
}

/// Replace every `{NAME}` occurrence in `s` with the value of the `NAME`
/// environment variable; unknown variables expand to the empty string (same
/// contract as `env::resolve_env` for MCP server envs).
pub fn expand_env_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + '{'.len_utf8()..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                out.push_str(&std::env::var(name).unwrap_or_default());
                rest = &after[close + '}'.len_utf8()..];
            }
            // Unterminated `{`: keep the remainder verbatim.
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object_deserializes_to_libsql_default() {
        let cfg: StorageConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.backend, StorageBackend::Libsql);
        assert_eq!(cfg.mysql, None);
        assert_eq!(cfg.starrocks, None);
        assert_eq!(cfg.dsn(), None);
    }

    #[test]
    fn parse_round_trips_every_backend_case_insensitively() {
        for b in [
            StorageBackend::Libsql,
            StorageBackend::Mysql,
            StorageBackend::Starrocks,
        ] {
            assert_eq!(StorageBackend::parse(b.as_str()), Some(b));
            assert_eq!(StorageBackend::parse(&b.as_str().to_uppercase()), Some(b));
        }
        assert_eq!(StorageBackend::parse("sqlite"), None);
        assert_eq!(StorageBackend::parse(""), None);
    }

    #[test]
    fn dsn_picks_the_configured_backend_field() {
        let cfg: StorageConfig = serde_json::from_str(
            r#"{"backend":"mysql","mysql":"mysql://h/db","starrocks":"sr://h/db"}"#,
        )
        .unwrap();
        assert_eq!(cfg.dsn().as_deref(), Some("mysql://h/db"));

        let cfg: StorageConfig = serde_json::from_str(
            r#"{"backend":"starrocks","mysql":"mysql://h/db","starrocks":"sr://h/db"}"#,
        )
        .unwrap();
        assert_eq!(cfg.dsn().as_deref(), Some("sr://h/db"));

        // Backend selected with no DSN configured -> None, not a panic.
        let cfg: StorageConfig = serde_json::from_str(r#"{"backend":"mysql"}"#).unwrap();
        assert_eq!(cfg.dsn(), None);
    }

    #[test]
    fn expand_env_vars_replaces_refs_and_keeps_plain_text() {
        // Unique var name: env mutation is process-global and tests run in
        // parallel, so a dedicated key avoids interference.
        std::env::set_var("OPENCODER_TEST_STORAGE_DSN", "secret-db");
        let out = expand_env_vars("mysql://user:{OPENCODER_TEST_STORAGE_DSN}@host/db");
        assert_eq!(out, "mysql://user:secret-db@host/db");
        std::env::remove_var("OPENCODER_TEST_STORAGE_DSN");

        assert_eq!(expand_env_vars("plain text stays"), "plain text stays");
        assert_eq!(expand_env_vars("{UNKNOWN_VAR_XYZ}"), "");
        assert_eq!(expand_env_vars("trail{open"), "trail{open");
    }
}
