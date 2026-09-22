//! Typed error markers for the brain crate.
//!
//! The runtime reports every failure through `anyhow::Error`, but two failure
//! classes need a machine-checkable identity: upstream embedding outages and
//! updates of an unknown capability id. The web layer maps the former to HTTP
//! 502 ("bad gateway — embedding backend down") and the latter to 404, while
//! every other error stays 400/500 — so both splits must be exact, and typed
//! markers (`downcast_ref`) replace the former substring probes on error
//! chains.

/// Marker error: every upstream embedding failure (HTTP error, cardinality
/// mismatch, empty vector) is carried as this typed error inside
/// `anyhow::Error`, so consumers (the web layer) can match on the type via
/// `downcast_ref` instead of substring-matching error chains.
///
/// `Display` keeps the historical `"embedding failed: {detail}"` shape so
/// logs and HTTP error bodies read identically before/after the typed
/// switch; `detail` folds the upstream `{:#}` chain (or the synthesized
/// reason for cardinality/emptiness violations).
#[derive(Debug, Clone)]
pub struct EmbeddingFailed {
    /// Folded upstream chain or synthesized reason, without the
    /// `"embedding failed: "` prefix (Display adds it).
    pub detail: String,
}

impl EmbeddingFailed {
    /// Build the marker with the given detail.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for EmbeddingFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "embedding failed: {}", self.detail)
    }
}

impl std::error::Error for EmbeddingFailed {}

/// Marker error: `Runtime::update_capability` on an unknown id. Carried as
/// this typed error inside `anyhow::Error`, so consumers (the web layer) can
/// match on the type via `downcast_ref` instead of substring-matching error
/// chains. Post-write invariant violations ("not found after insert/update")
/// stay plain anyhow strings — those are 500-class bugs, never 404.
///
/// `Display` keeps the historical `"brain capability not found: {id}"` shape
/// so logs and HTTP error bodies read identically before/after the typed
/// switch.
#[derive(Debug, Clone)]
pub struct BrainNotFound {
    /// The id the caller tried to update.
    pub id: String,
}

impl std::fmt::Display for BrainNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "brain capability not found: {}", self.id)
    }
}

impl std::error::Error for BrainNotFound {}
