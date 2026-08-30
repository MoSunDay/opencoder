//! Deterministic test scaffolding for the analyzer pipeline (test-only).
//!
//! Replaces rippy's (MIT, https://github.com/mpecan/rippy) `Config::empty()` +
//! `resolve::tests::MockLookup` setup: no config layer exists here, so tests
//! inject just a variable lookup.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::analyzer::Analyzer;
use crate::environment::Environment;
use crate::resolve::VarLookup;
use crate::verdict::Verdict;

/// A configurable in-memory [`VarLookup`] for tests.
#[derive(Default)]
pub(crate) struct MockLookup {
    vars: HashMap<String, String>,
}

impl MockLookup {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }
}

impl VarLookup for MockLookup {
    fn lookup(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }
}

/// Build an analyzer over `cwd` with the given variable lookup.
///
/// `from_env` cannot actually fail (the parser is stateless); the retry ladder
/// exists only so this helper can hand back a value without `unwrap` while the
/// assert fails the test on any real failure.
#[must_use]
pub(crate) fn analyzer_for_test(cwd: PathBuf, lookup: MockLookup) -> Analyzer {
    build_analyzer(cwd, lookup, 0)
}

fn build_analyzer(cwd: PathBuf, lookup: MockLookup, attempt: u8) -> Analyzer {
    let env = Environment::for_test(cwd).with_var_lookup(Box::new(lookup));
    let built = Analyzer::from_env(env);
    assert!(
        built.is_ok(),
        "analyzer init failed: {:?}",
        built.as_ref().err()
    );
    built.unwrap_or_else(|err| {
        assert!(attempt < 2, "analyzer init kept failing: {err}");
        build_analyzer(PathBuf::from("/"), MockLookup::new(), attempt + 1)
    })
}

/// Classify `command` under `cwd` with an injected variable lookup.
pub(crate) fn analyze_with(cwd: PathBuf, lookup: MockLookup, command: &str) -> Verdict {
    let mut analyzer = analyzer_for_test(cwd, lookup);
    analyzer
        .analyze(command)
        .unwrap_or_else(|err| Verdict::ask(format!("analysis failed: {err}")))
}
