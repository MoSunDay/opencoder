//! The core analysis engine: parses a command and produces a safety verdict.
//!
//! Ported from rippy `src/analyzer.rs` (MIT, https://github.com/mpecan/rippy).
//! Sandbox deltas: every config/CC-rule/trace layer is gone, so the flow is
//! parse -> strip env prefix -> walk the tree -> combine; a parse failure is
//! fail-closed `Ask`. Simple-command analysis lives in [`command`], control
//! flow in [`control_flow`], redirects/expansion/handler dispatch in
//! [`dispatch`].

use std::path::{Path, PathBuf};

use rable::{Node, NodeKind};

use crate::ast;
use crate::environment::Environment;
use crate::error::Error;
use crate::parser::BashParser;
use crate::resolve::LocalBinding;
use crate::verdict::{AllowReason, Verdict};

/// Reason for an arithmetic context carrying a substitution bash resolves by
/// running a command. Matches the wording `resolve` already emits for the same
/// hazard so the two paths read alike.
pub(crate) const EXPANSION_ASK: &str =
    "shell expansion (command substitution requires execution)";

const MAX_DEPTH: usize = 256;

/// Maximum number of AST nodes walked per command. Bounds tree *breadth*
/// where `MAX_DEPTH` only bounds tree height: a pathological input that
/// produces thousands of sibling nodes at shallow depth would otherwise
/// slip through. Analysis that hits the cap returns Ask.
const MAX_NODES: usize = 10_000;

/// Maximum length (bytes) of a resolved command string. Resolution that
/// would produce a longer string falls back to Ask, preventing pathological
/// expansions (e.g., variables that contain other expansions) from blowing up
/// memory.
pub(crate) const MAX_RESOLVED_LEN: usize = 16_384;

/// Maximum number of nested resolution passes. Each call to `try_resolve`
/// re-parses the resolved command and may resolve again; this cap is
/// independent of `MAX_DEPTH` (which bounds AST node nesting) and prevents
/// `A=$B; B=$C; C=$A` cycles from blowing the stack.
pub(crate) const MAX_RESOLUTION_DEPTH: usize = 8;

pub struct Analyzer {
    pub(crate) parser: BashParser,
    pub(crate) remote: bool,
    pub(crate) working_directory: PathBuf,
    /// Variable lookup used for static expansion resolution.
    /// Defaults to `EnvLookup` (real process environment); tests inject mocks.
    var_lookup: Box<dyn crate::resolve::VarLookup>,
    /// Tracks how many nested expansion-resolution passes have run for the
    /// current command. Bounded by `MAX_RESOLUTION_DEPTH` to prevent cycles.
    resolution_depth: usize,
    /// Remaining AST-node budget for the current `analyze` call. Reset to
    /// `MAX_NODES` at the top of every public `analyze` call and decremented
    /// once per `analyze_node` entry. Returns Ask when exhausted.
    node_budget: usize,
    /// Command-local variable bindings in effect for the node being analyzed:
    /// for/select loop variables, literal `VAR=val` prefixes, and prior literal
    /// assignments in a list. Managed with a strict checkpoint/truncate
    /// discipline so a binding never leaks past its lexical scope.
    pub(crate) locals: Vec<(String, LocalBinding)>,
}

impl Analyzer {
    /// Create a new analyzer from an [`Environment`] struct.
    ///
    /// # Errors
    ///
    /// Returns `Error::Parse` if the bash parser cannot be initialized.
    pub fn from_env(env: Environment) -> Result<Self, Error> {
        Ok(Self {
            parser: BashParser::new()?,
            remote: env.remote,
            working_directory: env.working_directory,
            var_lookup: env.var_lookup,
            resolution_depth: 0,
            node_budget: MAX_NODES,
            locals: Vec::new(),
        })
    }

    /// Analyze a shell command string and return a safety verdict.
    ///
    /// # Errors
    ///
    /// This method never errors on unparseable input: a command the parser
    /// refuses yields a fail-closed `Ask` verdict, so an
    /// unparseable-but-runnable command is gated rather than silently allowed.
    /// The `Result` signature is retained for call-site stability.
    pub fn analyze(&mut self, command: &str) -> Result<Verdict, Error> {
        let nodes = match self.parser.parse(command) {
            Ok(nodes) => nodes,
            Err(err) => return Ok(self.no_tree_ask(&err)),
        };
        // rippy matched its whole-string rule layers against the env-prefix
        // stripped command; the sandbox has no string layers, so the strip's
        // remaining effect is the fail-closed validation the tree walk gets
        // from `analyze_command`'s assignment guards (expansion, dangerous
        // name). see docs/security-invariants.md#env-prefix-strip
        let _stripped = ast::strip_env_prefix(command, &nodes);
        let cwd = self.working_directory.clone();
        self.node_budget = MAX_NODES;
        Ok(self.analyze_nodes(&nodes, &cwd, 0))
    }

    /// The Ask for a command that never became a tree. Input refused for its
    /// shape names the bound it broke, so the user sees why; everything else
    /// is a plain parse failure.
    fn no_tree_ask(&mut self, err: &Error) -> Verdict {
        if let Error::TooComplex(detail) = err {
            return Verdict::ask(format!("command is too complex to analyze: {detail}"));
        }
        Verdict::ask(format!("rippy could not parse this command ({err}); approve manually"))
    }

    pub(crate) fn analyze_nodes(&mut self, nodes: &[Node], cwd: &Path, depth: usize) -> Verdict {
        if nodes.is_empty() {
            return Verdict::allow(AllowReason::Empty);
        }
        let verdicts: Vec<Verdict> = nodes
            .iter()
            .map(|n| self.analyze_node(n, cwd, depth))
            .collect();
        Verdict::combine(&verdicts)
    }

    pub(crate) fn analyze_node(&mut self, node: &Node, cwd: &Path, depth: usize) -> Verdict {
        if depth > MAX_DEPTH {
            return Verdict::ask("nesting depth exceeded");
        }
        if self.node_budget == 0 {
            return Verdict::ask("ast node count exceeded");
        }
        self.node_budget -= 1;
        match &node.kind {
            NodeKind::Command { .. } => self.analyze_command(node, cwd, depth),
            NodeKind::Pipeline { commands, .. } => self.analyze_pipeline(commands, cwd, depth),
            NodeKind::List { items } => self.analyze_list(items, cwd, depth),
            NodeKind::If { .. }
            | NodeKind::While { .. }
            | NodeKind::Until { .. }
            | NodeKind::For { .. }
            | NodeKind::ForArith { .. }
            | NodeKind::Select { .. }
            | NodeKind::Case { .. }
            | NodeKind::BraceGroup { .. } => self.analyze_control_flow(node, cwd, depth),
            // `[[ ]]` joins the subshell arm because both carry redirects that a
            // body-only walk would leave unanalyzed (#197).
            NodeKind::Subshell { body, redirects }
            | NodeKind::ConditionalExpr { body, redirects } => {
                let mut verdicts = vec![self.analyze_node(body, cwd, depth + 1)];
                verdicts.extend(self.analyze_redirects(redirects, cwd, depth));
                Verdict::combine(&verdicts)
            }
            NodeKind::CommandSubstitution { command, .. } => {
                let inner = self.analyze_node(command, cwd, depth + 1);
                if ast::is_safe_heredoc_substitution(command) {
                    inner
                } else {
                    most_restrictive(inner, Verdict::ask("command substitution"))
                }
            }
            NodeKind::ProcessSubstitution { command, .. } => {
                let inner = self.analyze_node(command, cwd, depth + 1);
                most_restrictive(inner, Verdict::ask("command substitution"))
            }
            NodeKind::Function { .. } => Verdict::ask("function definition"),
            NodeKind::Negation { pipeline } | NodeKind::Time { pipeline, .. } => {
                self.analyze_node(pipeline, cwd, depth + 1)
            }
            NodeKind::HereDoc {
                quoted, content, ..
            } => Self::analyze_heredoc_node(*quoted, Some(content.as_str())),
            NodeKind::Coproc { command, .. } => self.analyze_node(command, cwd, depth + 1),
            NodeKind::ArithmeticCommand {
                redirects,
                raw_content,
                ..
            } => self.analyze_arithmetic_command(raw_content, redirects, cwd, depth),
            _ if ast::is_expansion_node(&node.kind) => Verdict::ask("shell expansion"),
            _ => Verdict::ask("unrecognized shell construct"),
        }
    }

    /// `(( ... ))`. rable's `expression` is an arithmetic AST it does not
    /// descend into for a nested substitution, so `raw_content` is the field
    /// that still carries a `$(...)`.
    fn analyze_arithmetic_command(
        &mut self,
        raw_content: &str,
        redirects: &[Node],
        cwd: &Path,
        depth: usize,
    ) -> Verdict {
        let mut verdicts = Vec::new();
        if ast::has_executing_substitution(raw_content) {
            verdicts.push(Verdict::ask(EXPANSION_ASK));
        }
        verdicts.extend(self.analyze_redirects(redirects, cwd, depth));
        if verdicts.is_empty() {
            // Pure arithmetic with no redirect runs nothing. Said here so that
            // `Verdict::combine` can keep failing closed on an empty slice.
            return Verdict::allow(AllowReason::Empty);
        }
        Verdict::combine(&verdicts)
    }

    fn analyze_pipeline(&mut self, commands: &[Node], cwd: &Path, depth: usize) -> Verdict {
        let has_unsafe_redirect = commands
            .iter()
            .any(|c| self.command_has_unsafe_redirect(c, cwd));

        let mut verdicts: Vec<Verdict> = commands
            .iter()
            .map(|cmd| self.analyze_node(cmd, cwd, depth + 1))
            .collect();

        if has_unsafe_redirect {
            verdicts.push(Verdict::ask("pipeline writes to file"));
        }

        Verdict::combine(&verdicts)
    }

    fn analyze_list(&mut self, items: &[rable::ListItem], cwd: &Path, depth: usize) -> Verdict {
        // Checkpoint so literal assignments registered for later items in this
        // list (`SCRATCH=/tmp; ls $SCRATCH`) never leak into a sibling scope.
        let checkpoint = self.locals.len();
        let mut verdicts = Vec::new();
        let mut current_cwd = cwd.to_owned();
        let mut is_harmless_fallback = false;

        for (i, item) in items.iter().enumerate() {
            let v = self.analyze_node(&item.command, &current_cwd, depth + 1);
            // Register standalone literal assignments so subsequent list items
            // (but nothing outside this list) can resolve them.
            self.register_list_bindings(&item.command);

            if let Some(dir) = extract_cd_target(&item.command) {
                current_cwd = if Path::new(&dir).is_absolute() {
                    PathBuf::from(&dir)
                } else {
                    current_cwd.join(&dir)
                };
            }

            // In `|| true` patterns, only include the fallback if it's non-trivial
            if is_harmless_fallback && v.decision == crate::verdict::Decision::Allow {
                is_harmless_fallback = false;
                continue;
            }
            is_harmless_fallback = false;

            if item.operator == Some(rable::ListOperator::Or)
                && items
                    .get(i + 1)
                    .is_some_and(|next| ast::is_harmless_fallback(&next.command))
            {
                is_harmless_fallback = true;
            }

            verdicts.push(v);
        }

        self.locals.truncate(checkpoint);
        Verdict::combine(&verdicts)
    }
}

#[path = "analyzer_command.rs"]
pub(crate) mod command;

#[path = "analyzer_control_flow.rs"]
pub(crate) mod control_flow;

#[path = "analyzer_dispatch.rs"]
pub(crate) mod dispatch;

/// Attach the resolved command string to a verdict. Appends the detail
/// to the reason (idempotent) and stores the resolved command in `resolved_command`.
pub(crate) fn annotate_with_resolution(mut v: Verdict, resolved: &str) -> Verdict {
    if !v.reason.contains("(resolved:") {
        v.reason = if v.reason.is_empty() {
            format!("(resolved: {resolved})")
        } else {
            format!("{} (resolved: {resolved})", v.reason)
        };
    }
    v.resolved_command = Some(resolved.to_string());
    v
}

fn extract_cd_target(node: &Node) -> Option<String> {
    let name = ast::command_name(node)?;
    if name != "cd" {
        return None;
    }
    let args = ast::command_args(node);
    args.first().cloned()
}

pub(crate) fn most_restrictive(a: Verdict, b: Verdict) -> Verdict {
    if a.decision >= b.decision { a } else { b }
}
