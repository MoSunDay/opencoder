//! Safety verdict types.
//!
//! Trimmed derivative of rippy's `verdict.rs` (MIT, https://github.com/mpecan/rippy):
//! the wire-serialization and config-rule provenance layers are dropped; the
//! decision semantics (`Allow` < `Ask` < `Deny`) and fail-closed combination are
//! preserved verbatim.

#[path = "allow_reason.rs"]
mod allow_reason;

pub use allow_reason::AllowReason;

/// The three possible safety decisions, ordered so `max()` gives the most restrictive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

/// A decision paired with a human-readable reason.
///
/// Construct one through [`Verdict::allow`], [`Verdict::ask`] or [`Verdict::deny`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub decision: Decision,
    /// Human-readable reason. For an approval it is initialized from
    /// `Display for AllowReason`; the analyzer may later append
    /// `(resolved: ...)` to it.
    pub reason: String,
    /// The fully-resolved command (after expansion of `$VAR`, `$'...'`,
    /// `$((...))`, etc.) when the analyzer was able to statically resolve all
    /// expansions. `None` when no resolution occurred or it failed.
    pub resolved_command: Option<String>,
}

impl Verdict {
    #[must_use]
    pub fn allow(reason: AllowReason) -> Self {
        Self {
            decision: Decision::Allow,
            reason: reason.to_string(),
            resolved_command: None,
        }
    }

    #[must_use]
    pub fn ask(reason: impl Into<String>) -> Self {
        Self {
            decision: Decision::Ask,
            reason: reason.into(),
            resolved_command: None,
        }
    }

    #[must_use]
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            decision: Decision::Deny,
            reason: reason.into(),
            resolved_command: None,
        }
    }

    /// Attach the resolved command string to this verdict.
    #[must_use]
    pub fn with_resolution(mut self, resolved: String) -> Self {
        self.resolved_command = Some(resolved);
        self
    }

    /// Combine a slice of verdicts into one, taking the most restrictive
    /// decision and the reason from whichever verdict drove that decision.
    ///
    /// The resolved command is preserved from the chosen verdict, or from any
    /// other verdict in the input if the chosen one has none -- so resolution
    /// info is never accidentally dropped during combination.
    ///
    /// An **empty** slice means the caller analyzed nothing, which is an error
    /// state rather than a safe one. The empty case fails closed to Ask.
    #[must_use]
    pub fn combine(verdicts: &[Self]) -> Self {
        let Some(most_restrictive) = verdicts.iter().max_by_key(|v| v.decision) else {
            return Self::ask("nothing to analyze");
        };
        let mut chosen = most_restrictive.clone();
        if chosen.resolved_command.is_none() {
            chosen.resolved_command = verdicts.iter().find_map(|v| v.resolved_command.clone());
        }
        chosen
    }
}

impl Default for Verdict {
    fn default() -> Self {
        Self::allow(AllowReason::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_ordering_is_allow_ask_deny() {
        assert!(Decision::Allow < Decision::Ask);
        assert!(Decision::Ask < Decision::Deny);
    }

    #[test]
    fn combine_takes_most_restrictive_and_keeps_resolution() {
        let allow = Verdict::allow(AllowReason::SimpleSafe("ls".into()));
        let ask = Verdict::ask("rm is destructive").with_resolution("rm -rf /x".into());
        let combined = Verdict::combine(&[allow, ask]);
        assert_eq!(combined.decision, Decision::Ask);
        assert_eq!(combined.resolved_command.as_deref(), Some("rm -rf /x"));
    }

    #[test]
    fn combine_empty_fails_closed() {
        let combined = Verdict::combine(&[]);
        assert_eq!(combined.decision, Decision::Ask);
    }
}
