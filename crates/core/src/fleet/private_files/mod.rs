//! Execution-scoped private inputs travel outside public requests and model prompts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod storage;
pub use storage::{materialize, runtime_image_digest};
#[cfg(test)]
mod tests;

pub const CAPABILITY: &str = "execution_private_files_v1";
pub const GUEST_ROOT: &str = "/run/opencoder-task";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivateExecutionContext {
    pub expires_at_ms: i64,
    pub image_digest: String,
    pub definition_sha256: String,
    pub files: BTreeMap<String, String>,
}

impl std::fmt::Debug for PrivateExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateExecutionContext([redacted])")
    }
}

impl PrivateExecutionContext {
    pub fn validate(&self, now_ms: i64) -> Result<(), &'static str> {
        if self.expires_at_ms <= now_ms
            || self.expires_at_ms.saturating_sub(now_ms) > 86_400_000
            || !self
                .image_digest
                .strip_prefix("sha256:")
                .is_some_and(digest)
            || !digest(&self.definition_sha256)
            || self.files.is_empty()
            || self.files.len() > 16
            || self.files.values().map(String::len).sum::<usize>() > 512 * 1024
            || self.files.keys().any(|name| !safe_name(name))
        {
            return Err("invalid or expired execution private files");
        }
        Ok(())
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
