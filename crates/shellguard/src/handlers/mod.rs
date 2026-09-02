//! Per-command handler layer: shared context, classification enum, trait and
//! registry.
//!
//! Trimmed derivative of rippy's `handlers/mod.rs` (MIT,
//! https://github.com/mpecan/rippy): the allow-surface/catalog reporting
//! (`allow_surface`, `surface.rs`) is dropped; classification semantics are
//! preserved.

mod args;
mod scope;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use args::{
    collect_flag_values, first_positional, get_flag_value, get_flag_values, has_clustered_short_flag,
    has_flag, has_flag_or_prefixed, has_glued_short_flag, is_sole_help_flag, positional_args,
    positional_operands,
};
pub(crate) use scope::{
    canonicalize_existing_ancestor, is_within_release_dir, is_within_safe_dir, normalize_path,
    operand_in_release,
};
pub(crate) use cd::resolve_target;

mod ansible;
mod cd;
mod cloud;
mod curl;
mod database;
mod docker;
mod env_xargs;
mod find;
mod getopt;
mod gh;
mod git;
mod git_guard;
mod git_subcommands;
mod helm;
mod mkdir;
mod node;
mod npm;
mod perl;
mod python;
mod python_tools;
mod ruby;
mod shell;
mod sandbox_release;
mod sed;
mod awk;
mod system;
mod task_runners;
mod unix_archive;
mod unix_misc;

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::verdict::AllowReason;

/// Context passed to handlers for classification.
pub(crate) struct HandlerContext<'a> {
    pub command_name: &'a str,
    pub args: &'a [String],
    pub working_directory: &'a Path,
    pub remote: bool,
    /// Extra directories path-based handlers (`cd`, `mkdir`, `git -C`) may
    /// enter/create in without prompting. The sandbox fills this with the
    /// release set beyond the built-in defaults (currently empty: the release
    /// set is exactly `/dev/null` + `/tmp`).
    pub safe_scopes: &'a [std::path::PathBuf],
}

/// Maximum file size (64 KB) for `read_file` -- prevents reading huge files.
const MAX_FILE_SIZE: u64 = 65_536;

impl HandlerContext<'_> {
    /// Get the first argument (typically a subcommand).
    pub(crate) fn subcommand(&self) -> &str {
        self.args.first().map_or("", String::as_str)
    }

    /// Get the Nth argument.
    pub(crate) fn arg(&self, n: usize) -> &str {
        self.args.get(n).map_or("", String::as_str)
    }

    /// Read a file's contents for informed classification.
    ///
    /// Returns `None` if the file can't be read (remote mode, missing,
    /// too large, or outside the working directory).
    pub(crate) fn read_file(&self, path: &str) -> Option<String> {
        if self.remote {
            return None;
        }
        let file_path = self.working_directory.join(path);
        let canonical = file_path.canonicalize().ok()?;
        let cwd_canonical = self.working_directory.canonicalize().ok()?;
        if !canonical.starts_with(&cwd_canonical) {
            return None;
        }
        let metadata = std::fs::metadata(&canonical).ok()?;
        if metadata.len() > MAX_FILE_SIZE {
            return None;
        }
        std::fs::read_to_string(&canonical).ok()
    }

    /// Construct a `HandlerContext` for unit tests with safe defaults.
    ///
    /// Defaults: `working_directory = /tmp`, `remote = false`,
    /// `safe_scopes = &[]`. Override any non-default field via struct-update
    /// syntax:
    /// `HandlerContext { remote: true, ..HandlerContext::test("cd", &args) }`.
    #[cfg(test)]
    pub(crate) fn test<'a>(command_name: &'a str, args: &'a [String]) -> HandlerContext<'a> {
        HandlerContext {
            command_name,
            args,
            working_directory: Path::new("/tmp"),
            remote: false,
            safe_scopes: &[],
        }
    }
}

/// The result of classifying a command.
#[derive(Debug, Clone)]
pub(crate) enum Classification {
    /// Auto-approve, carrying typed provenance (always
    /// [`AllowReason::Handler`] when minted by a handler).
    Allow(AllowReason),
    /// Needs user confirmation with description.
    Ask(String),
    /// Block with description. Wired to `Verdict::deny` in `apply_classification`.
    /// Only reachable once a deny path exists (rippy's deny rules were config
    /// driven and are not ported); kept for classification completeness.
    #[allow(dead_code)]
    Deny(String),
    /// Re-parse and analyze this inner command string.
    Recurse(String),
    /// Analyze the inner command string, then take the most restrictive of that
    /// verdict and the outer command's own classification.
    RecurseAtLeast(String, Box<Self>),
    /// Re-parse inner command with remote=true (for docker exec, kubectl exec).
    RecurseRemote(String),
    /// Approve the command itself, but route these redirect targets through
    /// the redirect safety pipeline (device targets, release-dir writes).
    WithRedirects(AllowReason, Vec<String>),
}

/// Trait for command handlers.
pub(crate) trait Handler: Send + Sync {
    fn commands(&self) -> &[&str];
    fn classify(&self, ctx: &HandlerContext) -> Classification;
}

/// A data-driven handler for commands with simple subcommand-based classification.
pub(crate) struct SubcommandHandler {
    cmds: &'static [&'static str],
    safe: &'static [&'static str],
    ask: &'static [&'static str],
    desc_prefix: &'static str,
}

impl SubcommandHandler {
    #[must_use]
    pub(crate) const fn new(
        cmds: &'static [&'static str],
        safe: &'static [&'static str],
        ask: &'static [&'static str],
        desc_prefix: &'static str,
    ) -> Self {
        Self {
            cmds,
            safe,
            ask,
            desc_prefix,
        }
    }
}

impl Handler for SubcommandHandler {
    fn commands(&self) -> &[&str] {
        self.cmds
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        let sub = ctx.args.first().map_or("", String::as_str);
        let desc = format!("{} {sub}", self.desc_prefix);

        // Check --help/--version first (only when it is the sole argument).
        if is_sole_help_flag(ctx.args, &["--help", "-h", "--version", "-V"]) {
            return Classification::Allow(AllowReason::handler(format!(
                "{} help/version",
                self.desc_prefix
            )));
        }

        if self.safe.contains(&sub) {
            Classification::Allow(AllowReason::handler(desc))
        } else if self.ask.contains(&sub) {
            Classification::Ask(desc)
        } else if sub.is_empty() {
            Classification::Ask(format!("{} (no subcommand)", self.desc_prefix))
        } else {
            Classification::Ask(desc)
        }
    }
}

static HANDLER_REGISTRY: LazyLock<HashMap<&'static str, &'static dyn Handler>> =
    LazyLock::new(build_registry);

fn build_registry() -> HashMap<&'static str, &'static dyn Handler> {
    let handlers: Vec<&'static dyn Handler> = vec![
        &cd::CD_HANDLER,
        &mkdir::MKDIR_HANDLER,
        &git::GIT_HANDLER,
        &docker::DOCKER_HANDLER,
        &node::NODE_HANDLER,
        &perl::PERL_HANDLER,
        &python::PYTHON_HANDLER,
        &ruby::RUBY_HANDLER,
        &shell::SHELL_HANDLER,
        &find::FIND_HANDLER,
        &curl::CURL_HANDLER,
        &npm::NPM_HANDLER,
        &helm::HELM_HANDLER,
        &gh::GH_HANDLER,
        &cloud::KUBECTL_HANDLER,
        &cloud::AWS_HANDLER,
        &cloud::GCLOUD_HANDLER,
        &cloud::AZ_HANDLER,
        &database::PSQL_HANDLER,
        &database::MYSQL_HANDLER,
        &database::SQLITE3_HANDLER,
        &sed::SED_HANDLER,
        &awk::AWK_HANDLER,
        &env_xargs::ENV_HANDLER,
        &env_xargs::XARGS_HANDLER,
        &unix_archive::TAR_HANDLER,
        &unix_misc::WGET_HANDLER,
        &python_tools::UV_HANDLER,
        &unix_archive::GZIP_HANDLER,
        &unix_archive::UNZIP_HANDLER,
        &unix_archive::SEVENZIP_HANDLER,
        &unix_misc::MKTEMP_HANDLER,
        &unix_misc::TEE_HANDLER,
        &unix_misc::SORT_HANDLER,
        &unix_misc::OUTPUT_FLAG_HANDLER,
        &unix_misc::HYPERFINE_HANDLER,
        &unix_misc::OPEN_HANDLER,
        &unix_misc::YQ_HANDLER,
        &unix_misc::DOS2UNIX_HANDLER,
        &python_tools::RUFF_HANDLER,
        &python_tools::BLACK_HANDLER,
        &system::FD_HANDLER,
        &system::DMESG_HANDLER,
        &system::IP_HANDLER,
        &system::IFCONFIG_HANDLER,
        &ansible::ANSIBLE_HANDLER,
        &task_runners::JUST_HANDLER,
        &task_runners::MISE_HANDLER,
        &task_runners::TOKF_HANDLER,
        &sandbox_release::SANDBOX_RELEASE_HANDLER,
    ];
    let mut registry = HashMap::with_capacity(handlers.len() * 2);
    for handler in handlers {
        for cmd in handler.commands() {
            registry.insert(*cmd, handler);
        }
    }
    registry
}

/// Look up a handler by command name.
#[must_use]
pub(crate) fn get_handler(command_name: &str) -> Option<&'static dyn Handler> {
    HANDLER_REGISTRY.get(command_name).copied()
}

/// Return the number of registered handler command names.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub(crate) fn handler_count() -> usize {
    HANDLER_REGISTRY.len()
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    /// The ported registry must cover exactly the command-name surface of
    /// rippy's `build_registry` (link_repos/rippy/src/handlers/mod.rs): 90
    /// unique command names over the same handler statics.
    #[test]
    fn registry_matches_rippy_command_surface() {
        // Sandbox delta count: 11 release/sandbox handlers beyond rippy's 90,
        // plus shuf/iconv/hyperfine promoted out of SIMPLE_SAFE into their own
        // handlers (#F4: argument-shell execution / -o file writes).
        assert_eq!(handler_count(), 90 + 11 + 3);
        for name in ["cd", "git", "kubectl", "sqlite3", "awk", "7zz", "python3.14", "tokf"] {
            assert!(get_handler(name).is_some(), "{name} missing from registry");
        }
        // Sandbox delta: rm/chmod ARE registered (the sandbox release handler
        // owns them); `sudo`/`shred` stay unhandled so they hit the fail-closed
        // default Ask.
        for name in ["sudo", "shred"] {
            assert!(get_handler(name).is_none(), "{name} must stay unregistered");
        }
        for name in ["rm", "mv", "cp", "touch", "ln", "chmod", "chown", "chgrp", "install", "truncate", "rmdir"] {
            assert!(
                get_handler(name).is_some(),
                "{name} must be registered by the sandbox release handler"
            );
        }
    }
}
