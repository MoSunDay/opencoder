//! Redirect analysis, handler dispatch and expansion re-classification.
//! Ported from rippy `src/analyzer_dispatch.rs` (MIT,
//! https://github.com/mpecan/rippy). Sandbox deltas: no self-protect or
//! config-redirect layers, the release set is `/dev/null` + `/tmp` (never the
//! cwd), and an unknown command always Asks.

use std::path::Path;

use rable::{Node, NodeKind};

use super::{
    Analyzer, EXPANSION_ASK, MAX_RESOLUTION_DEPTH, MAX_RESOLVED_LEN, annotate_with_resolution,
};
use crate::handlers::canonicalize_existing_ancestor;
use crate::allowlists;
use crate::ast;
use crate::handlers::{self, Classification, HandlerContext};
use crate::resolve;
use crate::verdict::{AllowReason, Verdict};

/// No extra scopes: the sandbox release set is exactly the built-in defaults.
const EMPTY_SCOPES: &[std::path::PathBuf] = &[];

impl Analyzer {
    pub(super) fn analyze_redirects(
        &mut self,
        redirects: &[Node],
        cwd: &Path,
        _depth: usize,
    ) -> Vec<Verdict> {
        let mut verdicts = Vec::new();
        for redir in redirects {
            match &redir.kind {
                NodeKind::Redirect { .. } => {
                    if let Some((op, target)) = ast::redirect_info(redir) {
                        verdicts.push(self.analyze_redirect(op, &target, cwd));
                    }
                }
                NodeKind::HereDoc {
                    quoted, content, ..
                } => {
                    verdicts.push(Self::analyze_heredoc_node(*quoted, Some(content.as_str())));
                }
                _ => {}
            }
        }
        verdicts
    }

    pub(super) fn classify_with_handler(
        &mut self,
        cmd_name: &str,
        args: &[String],
        cwd: &Path,
        depth: usize,
    ) -> Verdict {
        if let Some(handler) = handlers::get_handler(cmd_name) {
            let ctx = HandlerContext {
                command_name: cmd_name,
                args,
                working_directory: cwd,
                remote: self.remote,
                safe_scopes: EMPTY_SCOPES,
            };
            let classification = handler.classify(&ctx);
            return self.apply_classification(classification, cwd, depth);
        }

        self.default_verdict(cmd_name)
    }

    /// Combine a command-level verdict with the verdicts of its redirects
    /// (most-restrictive wins), so an allow rule / safe command cannot bypass the
    /// redirect safety pipeline (self-protect, safe-dir, deny rules).
    pub(super) fn with_redirects(
        &mut self,
        verdict: Verdict,
        redirects: &[Node],
        cwd: &Path,
    ) -> Verdict {
        // `analyze_redirects` ignores the depth argument (redirect targets are leaf
        // paths, not recursively analyzed commands), so a fixed 0 is fine here.
        let redirect_verdicts = self.analyze_redirects(redirects, cwd, 0);
        if redirect_verdicts.is_empty() {
            return verdict;
        }
        let mut all = vec![verdict];
        all.extend(redirect_verdicts);
        Verdict::combine(&all)
    }

    /// Run one redirect target through the write pipeline.
    pub(super) fn analyze_redirect(
        &mut self,
        op: ast::RedirectOp,
        target: &str,
        cwd: &Path,
    ) -> Verdict {
        self.redirect_verdict(op, target, cwd)
    }

    fn redirect_verdict(&self, op: ast::RedirectOp, target: &str, cwd: &Path) -> Verdict {
        // Before the read shortcut: `cat < "$(x)"` reads a file *named by* `x`.
        if ast::has_executing_substitution(target) {
            return Verdict::ask(EXPANSION_ASK);
        }
        if op == ast::RedirectOp::Read {
            return Verdict::allow(AllowReason::InputRedirect);
        }
        // `&>`/`>&` parse as `FdDup`; a path target is a real file write and must
        // run the write pipeline. see docs/security-invariants.md#fd-dup-remap
        let op = if op == ast::RedirectOp::FdDup {
            if ast::is_fd_dup_target(target) {
                return Verdict::allow(AllowReason::FdRedirect);
            }
            ast::RedirectOp::Write
        } else {
            op
        };
        if ast::is_safe_redirect_target(target) {
            return Verdict::allow(AllowReason::DeviceRedirect(target.to_owned()));
        }
        // Release-set writes auto-approve (rippy's user rules ran before this;
        // the sandbox has none).
        if matches!(op, ast::RedirectOp::Write | ast::RedirectOp::Append)
            && self.is_safe_write_target(target, cwd)
        {
            return Verdict::allow(AllowReason::SafeDirWrite(target.to_owned()));
        }
        Verdict::ask(format!("redirect to {target}"))
    }

    /// Returns `true` only for statically-known write targets that resolve inside
    /// the trusted safe-dir set (declared scopes or default safe dirs). Relative
    /// targets resolve against `cwd`. Conservative by construction and guarded by
    /// a cwd exclusion and a symlink re-check for the world-writable defaults —
    /// see docs/security-invariants.md#tmp-symlink.
    pub(super) fn is_safe_write_target(&self, target: &str, cwd: &Path) -> bool {
        let target = resolve::strip_outer_quotes(target);
        if ast::has_shell_expansion_pattern(&target)
            || target.starts_with('~')
            || target.contains(['*', '?', '['])
        {
            return false;
        }
        let raw = Path::new(&target);
        let resolved = if raw.is_absolute() {
            handlers::normalize_path(raw)
        } else {
            handlers::normalize_path(&cwd.join(raw))
        };
        // Project files keep asking even though a cwd could live under a
        // release dir: in sandbox mode the working directory is never released.
        if resolved.starts_with(handlers::normalize_path(cwd)) {
            return false;
        }
        // The release dirs are world-writable: require both the logical target
        // and its symlink-resolved real path to stay inside them.
        handlers::is_within_release_dir(&resolved)
            && handlers::is_within_release_dir(&canonicalize_existing_ancestor(&resolved))
    }

    /// Scope-aware unsafe-redirect check: a command has an unsafe write/append
    /// redirect only when its target is neither inherently safe (`/dev/null`)
    /// nor inside the trusted safe-dir set.
    pub(super) fn command_has_unsafe_redirect(&self, node: &Node, cwd: &Path) -> bool {
        let NodeKind::Command { redirects, .. } = &node.kind else {
            return false;
        };
        redirects.iter().any(|r| {
            let Some((op, target)) = ast::redirect_info(r) else {
                return false;
            };
            matches!(op, ast::RedirectOp::Write | ast::RedirectOp::Append)
                && !ast::is_safe_redirect_target(&target)
                && !self.is_safe_write_target(&target, cwd)
        })
    }

    pub(super) fn analyze_heredoc_node(quoted: bool, content: Option<&str>) -> Verdict {
        if quoted {
            return Verdict::allow(AllowReason::Heredoc);
        }
        if let Some(body) = content {
            if ast::has_shell_expansion_pattern(body) {
                return Verdict::ask("heredoc with expansion");
            }
        }
        Verdict::allow(AllowReason::Heredoc)
    }

    pub(super) fn analyze_inner_command(
        &mut self,
        inner: &str,
        cwd: &Path,
        depth: usize,
    ) -> Verdict {
        let Ok(nodes) = self.parser.parse(inner) else {
            return Verdict::ask("unparseable inner command");
        };
        self.analyze_nodes(&nodes, cwd, depth)
    }

    /// Attempt to statically resolve any shell expansions in `words` and
    /// re-classify the resolved command through the full pipeline.
    ///
    /// Returns:
    /// - `None` when there are no expansions to resolve (caller proceeds normally)
    /// - `Some(verdict)` when expansions were present:
    ///   - On unresolvable expansions, an `Ask` verdict with a diagnostic reason
    ///   - On command-position dynamic execution (`$cmd args`), an `Ask` verdict
    ///     regardless of whether resolution succeeded
    ///   - Otherwise, the verdict of re-analyzing the resolved command
    ///     (annotated with the resolved form for transparency)
    pub(super) fn try_resolve(
        &mut self,
        words: &[Node],
        cwd: &Path,
        depth: usize,
    ) -> Option<Verdict> {
        if !ast::has_expansions_in_slices(words, &[]) {
            return None;
        }
        // Bail out on runaway resolution (also catches cycles like `A=$B; B=$A`).
        if self.resolution_depth >= MAX_RESOLUTION_DEPTH {
            return Some(Verdict::ask("shell expansion (resolution depth exceeded)"));
        }
        let resolved = {
            let scoped = resolve::ScopedLookup::new(&self.locals, self.var_lookup.as_ref());
            resolve::resolve_command_args(words, &scoped)
        };
        // Set-but-unknown value in argument position: never fabricated, and the
        // relaxed allow is gated on no other word being unresolvable.
        // see docs/security-invariants.md#dynamic-arg
        if resolved.arg_position_dynamic
            && !resolved.command_position_dynamic
            && resolved.failure_reason.is_none()
        {
            return Some(self.dynamic_arg_verdict(words));
        }
        let Some(args) = resolved.args else {
            let reason = resolved.failure_reason.map_or_else(
                || "shell expansion".to_string(),
                |r| format!("shell expansion ({r})"),
            );
            return Some(Verdict::ask(reason));
        };
        let resolved_command = resolve::shell_join(&args);
        // Refuse to materialize pathologically large resolved commands.
        if resolved_command.len() > MAX_RESOLVED_LEN {
            return Some(Verdict::ask(format!(
                "shell expansion (resolved command exceeds {MAX_RESOLVED_LEN}-byte limit)"
            )));
        }
        if resolved.command_position_dynamic {
            return Some(
                Verdict::ask(format!("dynamic command (resolved: {resolved_command})"))
                    .with_resolution(resolved_command),
            );
        }
        // Track nesting around the recursive analyze_inner_command call.
        self.resolution_depth += 1;
        let inner = self.analyze_inner_command(&resolved_command, cwd, depth + 1);
        self.resolution_depth -= 1;
        Some(annotate_with_resolution(inner, &resolved_command))
    }

    /// Verdict for a command with a dynamic-known argument (`$loopvar`, `$?`).
    ///
    /// SECURITY INVARIANT: this is the *only* place a dynamic argument relaxes
    /// the verdict, and it does so strictly for the pure-reader subset of
    /// `SIMPLE_SAFE` (see [`allowlists::is_dynamic_arg_safe`]), whose safety does
    /// not depend on argument values (`cat`/`echo`/`wc`/`ls`). Commands that can
    /// act on an argument value — pagers that spawn subshells (`less`/`man`),
    /// preview-executing finders (`fzf`), and state-changing commands
    /// (`mount`/`stty`) — are excluded, as is any handler command (`rm`,
    /// `git`, ...), because a set-but-unknown value could otherwise hide an
    /// injected dangerous flag or path.
    pub(super) fn dynamic_arg_verdict(&mut self, words: &[Node]) -> Verdict {
        let Some(name) = ast::command_name_from_words(words) else {
            return Verdict::ask("shell expansion ($VAR dynamic)");
        };
        if allowlists::is_dynamic_arg_safe(name) {
            Verdict::allow(AllowReason::DynamicArgSafe(name.to_owned()))
        } else {
            Verdict::ask("shell expansion ($VAR dynamic)")
        }
    }

    pub(super) fn apply_classification(
        &mut self,
        class: Classification,
        cwd: &Path,
        depth: usize,
    ) -> Verdict {
        match class {
            Classification::Allow(reason) => Verdict::allow(reason),
            Classification::Ask(desc) => Verdict::ask(desc),
            Classification::Deny(desc) => Verdict::deny(desc),
            Classification::Recurse(inner) => self.analyze_inner_command(&inner, cwd, depth),
            Classification::RecurseAtLeast(inner, outer) => {
                let outer = self.apply_classification(*outer, cwd, depth);
                let inner = self.analyze_inner_command(&inner, cwd, depth);
                // Outer last so an equal-decision tie reports its reason, which
                // names the flag that spawned the program.
                Verdict::combine(&[inner, outer])
            }
            Classification::RecurseRemote(inner) => {
                let prev_remote = self.remote;
                self.remote = true;
                let v = self.analyze_inner_command(&inner, cwd, depth);
                self.remote = prev_remote;
                v
            }
            Classification::WithRedirects(reason, targets) => {
                let mut verdicts = vec![Verdict::allow(reason)];
                for target in &targets {
                    verdicts.push(self.analyze_redirect(ast::RedirectOp::Write, target, cwd));
                }
                Verdict::combine(&verdicts)
            }
        }
    }

    /// Fail-closed default: an unregistered command always Asks (rippy let a
    /// configured default action or allow decide; the sandbox has neither).
    pub(super) fn default_verdict(&mut self, cmd_name: &str) -> Verdict {
        Verdict::ask(format!("{cmd_name} (unknown command)"))
    }
}
