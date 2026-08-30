//! Simple-command analysis: assignment guards, allowlists, handler dispatch.
//!
//! Ported from rippy `src/analyzer.rs` (MIT, https://github.com/mpecan/rippy).
//! Sandbox delta: the per-leaf config/CC string-rule layer is gone — a leaf
//! falls straight through the allowlists to the handler registry or the
//! fail-closed default.

use std::path::Path;

use rable::{Node, NodeKind};

use super::Analyzer;
use crate::allowlists;
use crate::ast;
use crate::handlers::is_sole_help_flag;
use crate::resolve::LocalBinding;
use crate::verdict::{AllowReason, Verdict};

impl Analyzer {
    /// Push literal `VAR=val` bindings from a command's assignment nodes onto
    /// the locals stack. Non-literal values (expansions) are skipped — they are
    /// never bound and remain subject to the assignment-expansion guard.
    pub(crate) fn push_literal_bindings(&mut self, assignments: &[Node]) {
        for a in assignments {
            if let Some((name, val)) = ast::literal_assignment(a) {
                self.locals.push((name, LocalBinding::Literal(val)));
            } else if let Some(name) = ast::append_assignment_name(a) {
                // see rippy docs/security-invariants.md#append-assignment-shadow
                self.locals.push((name, LocalBinding::Dynamic));
            }
        }
    }

    /// Register the literal assignments of a *standalone* assignment command
    /// (`SCRATCH=/tmp/x` with no command word) so later items in the same list
    /// can resolve them. Commands that carry a word bind their prefix only for
    /// their own duration (handled in the `Command` arm), so they are skipped.
    pub(crate) fn register_list_bindings(&mut self, node: &Node) {
        let NodeKind::Command {
            assignments, words, ..
        } = &node.kind
        else {
            return;
        };
        if words.is_empty() {
            self.push_literal_bindings(assignments);
        }
    }

    /// Analyze a simple `Command` node.
    ///
    /// Applies the assignment-expansion guard (`x=$(cmd)` → Ask), then binds any
    /// literal `VAR=val` prefix for the duration of this one command so
    /// `VAR=val cmd $VAR` resolves within it, and unwinds the binding afterward.
    pub(super) fn analyze_command(&mut self, node: &Node, cwd: &Path, depth: usize) -> Verdict {
        let NodeKind::Command {
            words,
            redirects,
            assignments,
        } = &node.kind
        else {
            // Unreachable: only dispatched on `NodeKind::Command`. Fail closed
            // for a security tool so a future dispatch change cannot silently
            // approve an unhandled node kind.
            return Verdict::ask("internal: non-command node in analyze_command");
        };
        if Self::assignment_has_expansion(assignments) {
            return Verdict::ask("assignment with expansion");
        }
        if let Some(name) = Self::dangerous_assignment_name(assignments) {
            return Verdict::ask(format!(
                "dangerous env-var assignment ({name} is code-influencing)"
            ));
        }
        let checkpoint = self.locals.len();
        self.push_literal_bindings(assignments);
        let v = self.analyze_command_node(words, redirects, cwd, depth);
        self.locals.truncate(checkpoint);
        v
    }

    /// Returns `true` if any `NAME=VALUE` assignment on a simple command has a
    /// shell expansion in its value (e.g. a command substitution or backticks).
    ///
    /// Assignment values are not otherwise inspected by the analyzer, so this
    /// guard — applied to every simple command, including those nested in
    /// pipelines and lists — forces such commands to Ask. Literal assignments
    /// (`FOO=bar ls`) contain no expansion and pass through unaffected.
    fn assignment_has_expansion(assignments: &[Node]) -> bool {
        assignments.iter().any(ast::has_expansions)
    }

    /// Returns the name of the first assignment on a simple command that sets a
    /// code-influencing variable (`LD_PRELOAD`, `GIT_SSH_COMMAND`,
    /// `GIT_CONFIG_*`, ...). Such a literal prefix turns an otherwise-safe
    /// command into arbitrary code execution, so the analyzer Asks before the
    /// safe-command fast path or any handler can approve it.
    fn dangerous_assignment_name(assignments: &[Node]) -> Option<String> {
        assignments.iter().find_map(|a| {
            ast::literal_assignment(a)
                .map(|(n, _)| n)
                .or_else(|| ast::append_assignment_name(a))
                .filter(|n| ast::is_dangerous_env_name(n))
        })
    }

    pub(super) fn analyze_command_node(
        &mut self,
        words: &[Node],
        redirects: &[Node],
        cwd: &Path,
        depth: usize,
    ) -> Verdict {
        if let Some(resolved_verdict) = self.try_resolve(words, cwd, depth) {
            // combine (not most_restrictive) keeps the resolved_command field even
            // when a redirect verdict dominates the decision.
            let mut verdicts = vec![resolved_verdict];
            verdicts.extend(self.analyze_redirects(redirects, cwd, depth));
            return Verdict::combine(&verdicts);
        }

        let Some(cmd_name) = self.resolved_command_name(words) else {
            return Verdict::allow(AllowReason::EmptyCommand);
        };
        let args = ast::command_args_from_words(words);

        // The unwrapped inner command never sees the outer node's redirects.
        // See rippy docs/security-invariants.md#wrapper-redirects.
        if allowlists::is_wrapper(&cmd_name) {
            let inner_args = allowlists::wrapper_inner_args(&cmd_name, &args);
            let verdict = if inner_args.is_empty() {
                Verdict::allow(AllowReason::Wrapper(cmd_name.clone()))
            } else {
                self.analyze_inner_command(&inner_args.join(" "), cwd, depth)
            };
            return self.with_redirects(verdict, redirects, cwd);
        }

        if allowlists::is_simple_safe(&cmd_name) {
            return self.with_redirects(
                Verdict::allow(AllowReason::SimpleSafe(cmd_name.clone())),
                redirects,
                cwd,
            );
        }

        // Short-circuit to Allow ONLY when the help/version flag is the sole arg;
        // matching it anywhere let a dangerous operand ride along (#149). Bare `-h`
        // is dropped (commands overload it as `-h <host>`), so a lone `-h` Asks.
        if is_sole_help_flag(&args, allowlists::SOLE_HELP_FLAGS) {
            return Verdict::allow(AllowReason::HelpFlag(cmd_name.clone()));
        }

        let handler_verdict = self.classify_with_handler(&cmd_name, &args, cwd, depth);
        self.with_redirects(handler_verdict, redirects, cwd)
    }

    /// Resolve a command's name. rippy routed this through its alias table;
    /// the sandbox has no aliases, so the raw name is used verbatim.
    fn resolved_command_name(&self, words: &[Node]) -> Option<String> {
        ast::command_name_from_words(words).map(str::to_owned)
    }
}
