//! External environment dependencies for the analysis pipeline.
//!
//! Trimmed derivative of rippy's `environment.rs` (MIT,
//! https://github.com/mpecan/rippy): the `home` and `verbose` knobs are
//! dropped — sandbox classification has no config file to look up and no
//! tracing to emit. Production code builds this via
//! [`Environment::from_system`]; tests inject a deterministic
//! [`crate::resolve::VarLookup`] through [`Environment::with_var_lookup`].

use std::path::PathBuf;

use crate::resolve::{EnvLookup, VarLookup};

/// Groups the values that come from the OS environment so they can be
/// overridden in tests without manipulating env vars.
pub struct Environment {
    /// Working directory for the analysis (usually `std::env::current_dir()`).
    pub working_directory: PathBuf,

    /// Whether the command originates from a remote context (e.g. `docker exec`).
    pub remote: bool,

    /// Variable lookup for static expansion resolution.
    /// Defaults to `EnvLookup` (real `std::env::var`).
    pub(crate) var_lookup: Box<dyn VarLookup>,
}

impl Environment {
    /// Build from the real system environment.
    #[must_use]
    pub fn from_system(working_directory: PathBuf, remote: bool) -> Self {
        Self {
            working_directory,
            var_lookup: Box::new(EnvLookup),
            remote,
        }
    }

    /// Build an isolated environment for tests: not remote, real env-var
    /// lookup for variable resolution.
    #[must_use]
    pub fn for_test(working_directory: PathBuf) -> Self {
        Self {
            working_directory,
            var_lookup: Box::new(EnvLookup),
            remote: false,
        }
    }

    /// Override the variable lookup (builder pattern). Deterministic test
    /// injection point, kept available outside `cfg(test)` so a future
    /// executor can drive the pipeline with a non-env lookup.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub(crate) fn with_var_lookup(mut self, var_lookup: Box<dyn VarLookup>) -> Self {
        self.var_lookup = var_lookup;
        self
    }
}
