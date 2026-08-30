//! Error type for the classification pipeline.
//!
//! Trimmed derivative of rippy's `error.rs` (MIT, https://github.com/mpecan/rippy):
//! only the two variants the parse/AST pipeline can produce survive.

/// Errors that can occur while classifying a command.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("parse error: {0}")]
    Parse(String),

    /// Input whose shape would drive the recursive-descent parser off the stack.
    /// see rippy docs/security-invariants.md#parser-stack-bound
    #[error("input too complex: {0}")]
    TooComplex(String),
}
