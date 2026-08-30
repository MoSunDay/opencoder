//! `shellguard` -- shell command safety classifier for sandbox mode.
//!
//! Classification core extracted from [rippy](https://github.com/mpecan/rippy)
//! (MIT license, copyright the rippy authors) and adapted to the sandbox
//! policy: every risk-bearing write is blocked; writes targeting `/dev/null`
//! or `/tmp` are released; the working directory is NOT a release set.
//! `Allow` passes; `Ask`/`Deny` block.
//!
//! Pipeline: [`nesting`] bounds the input shape, [`parser`] turns it into a
//! rable AST, [`ast`] classifies nodes, [`resolve`] statically resolves shell
//! expansions, and [`analyzer`] walks the tree down into the per-command
//! [`handlers`] registry. Everything funnels through [`classify`].

pub mod allowlists;
pub mod analyzer;
pub mod ast;
pub mod environment;
pub mod error;
pub mod handlers;
pub mod nesting;
pub mod node_safety;
pub mod parser;
pub mod perl_safety;
pub mod python_safety;
pub mod resolve;
pub mod ruby_safety;
pub mod sql;
pub mod verdict;

pub use analyzer::Analyzer;
pub use environment::Environment;
pub use resolve::{EnvLookup, VarLookup};
pub use verdict::{AllowReason, Decision, Verdict};

/// Classify a shell command under the sandbox policy.
pub fn classify(command: &str) -> Verdict {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    let mut analyzer = match Analyzer::from_env(Environment::from_system(cwd, false)) {
        Ok(analyzer) => analyzer,
        // The parser is stateless and never fails; treat it as unparseable.
        Err(err) => return Verdict::ask(format!("unparseable command: {err}")),
    };
    analyzer
        .analyze(command)
        .unwrap_or_else(|err| Verdict::ask(format!("unparseable command: {err}")))
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "analyzer_sandbox_tests.rs"]
mod analyzer_sandbox_tests;

#[cfg(test)]
#[path = "analyzer_pipeline_tests.rs"]
mod analyzer_pipeline_tests;
