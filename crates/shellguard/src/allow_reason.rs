//! Why a command was approved -- the typed provenance of every `Allow`.
//!
//! Trimmed derivative of rippy's `allow_reason.rs` (MIT,
//! https://github.com/mpecan/rippy): config-rule / CC-permission / catalog
//! variants are dropped; the handler-path variants and their `Display`
//! renderings (which become the wire `reason` string) are preserved verbatim.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowReason {
    /// Nothing to report: an empty node list or a construct the walker does not gate.
    Empty,
    /// A command node with no command name.
    EmptyCommand,
    /// The command is in the `SIMPLE_SAFE` allowlist.
    SimpleSafe(String),
    /// A wrapper command (`env`, `nohup`, ...) invoked with no inner command.
    Wrapper(String),
    /// `--help`/`--version` was the sole argument.
    HelpFlag(String),
    /// A pure-reader allowlist command carrying a dynamically-known argument.
    DynamicArgSafe(String),
    /// An input (`<`) redirect, which cannot write.
    InputRedirect,
    /// A file-descriptor duplication (`2>&1`), which targets no path.
    FdRedirect,
    /// A redirect to an inherently safe device (`/dev/null`, `/dev/stdout`, ...).
    DeviceRedirect(String),
    /// A redirect whose target normalizes inside a released directory.
    SafeDirWrite(String),
    /// A quoted heredoc body: literal text, no expansion, no execution.
    Heredoc,
    /// Approved by a per-command handler.
    Handler(String),
}

impl AllowReason {
    /// Build a handler-provenance reason from a handler's description.
    pub fn handler(detail: impl Into<String>) -> Self {
        Self::Handler(detail.into())
    }
}

impl fmt::Display for AllowReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => Ok(()),
            Self::EmptyCommand => f.write_str("empty command"),
            Self::SimpleSafe(cmd) => write!(f, "{cmd} is safe"),
            Self::Wrapper(cmd) => write!(f, "{cmd} (no inner command)"),
            Self::HelpFlag(cmd) => write!(f, "{cmd} help/version"),
            Self::DynamicArgSafe(cmd) => write!(f, "{cmd} is safe (dynamic arg)"),
            Self::InputRedirect => f.write_str("input redirect"),
            Self::FdRedirect => f.write_str("fd redirect"),
            Self::DeviceRedirect(target) => write!(f, "redirect to {target}"),
            Self::SafeDirWrite(target) => write!(f, "redirect to {target} (safe dir)"),
            Self::Heredoc => f.write_str("heredoc"),
            Self::Handler(detail) => f.write_str(detail),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_renders_wire_reason_strings() {
        assert_eq!(AllowReason::SimpleSafe("ls".into()).to_string(), "ls is safe");
        assert_eq!(
            AllowReason::SafeDirWrite("/tmp/a.log".into()).to_string(),
            "redirect to /tmp/a.log (safe dir)"
        );
        assert_eq!(
            AllowReason::DeviceRedirect("/dev/null".into()).to_string(),
            "redirect to /dev/null"
        );
        assert_eq!(AllowReason::FdRedirect.to_string(), "fd redirect");
    }
}
