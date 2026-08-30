//! AST helpers over rable's parse trees: command names/arguments, redirect
//! metadata, shell-expansion detection and the safe-heredoc idiom.
//! Ported from rippy `src/ast.rs` (MIT, https://github.com/mpecan/rippy);
//! quoting lives in [`quote`] and env-prefix/assignment helpers in [`env`].

use rable::{Node, NodeKind};

use crate::allowlists;

/// The operator used in a file redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectOp {
    /// `>` — write (truncate)
    Write,
    /// `>>` — append
    Append,
    /// `<` — read
    Read,
    /// `>&` or `&>` — file descriptor duplication
    FdDup,
    /// Anything else
    Other,
}

/// Extract the command name from a word slice.
#[must_use]
pub fn command_name_from_words(words: &[Node]) -> Option<&str> {
    words.first().and_then(word_value)
}

/// Extract the command name from a `Command` node.
#[must_use]
pub fn command_name(node: &Node) -> Option<&str> {
    let NodeKind::Command { words, .. } = &node.kind else {
        return None;
    };
    command_name_from_words(words)
}

/// Extract command arguments from a word slice (all words after the name).
#[must_use]
pub fn command_args_from_words(words: &[Node]) -> Vec<String> {
    words.iter().skip(1).map(node_text).collect()
}

/// Extract command arguments from a `Command` node.
#[must_use]
pub fn command_args(node: &Node) -> Vec<String> {
    let NodeKind::Command { words, .. } = &node.kind else {
        return Vec::new();
    };
    command_args_from_words(words)
}

/// Extract the redirect operator and target from a `Redirect` node.
#[must_use]
pub fn redirect_info(node: &Node) -> Option<(RedirectOp, String)> {
    let NodeKind::Redirect { op, target, .. } = &node.kind else {
        return None;
    };
    let redirect_op = match op.as_str() {
        ">" => RedirectOp::Write,
        ">>" => RedirectOp::Append,
        "<" | "<<<" => RedirectOp::Read,
        "&>" | ">&" => RedirectOp::FdDup,
        _ => RedirectOp::Other,
    };
    Some((redirect_op, node_text(target)))
}

/// Check whether a node contains command or process substitutions.
///
/// Rable keeps `$(...)` and backtick substitutions as literal text in word
/// values, so we check word values for expansion patterns.
#[must_use]
pub fn has_expansions(node: &Node) -> bool {
    has_expansions_kind(&node.kind)
}

/// Check for expansions in word and redirect slices.
#[must_use]
pub fn has_expansions_in_slices(words: &[Node], redirects: &[Node]) -> bool {
    words.iter().any(has_expansions) || redirects.iter().any(has_expansions)
}

/// Returns `true` if the node kind is itself a shell expansion.
///
/// This is the single source of truth for which `NodeKind` variants
/// represent expansions. Used by both `has_expansions_kind` (AST walking)
/// and `analyze_node` (verdict generation).
#[must_use]
pub const fn is_expansion_node(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::CommandSubstitution { .. }
            | NodeKind::ProcessSubstitution { .. }
            | NodeKind::ParamExpansion { .. }
            | NodeKind::ParamIndirect { .. }
            | NodeKind::ParamLength { .. }
            | NodeKind::AnsiCQuote { .. }
            | NodeKind::LocaleString { .. }
            | NodeKind::ArithmeticExpansion { .. }
            | NodeKind::BraceExpansion { .. }
    )
}

fn has_expansions_kind(kind: &NodeKind) -> bool {
    if is_expansion_node(kind) {
        return true;
    }
    match kind {
        NodeKind::Word { value, parts, .. } => {
            // A backtick inside a double-quoted part stays literal text in
            // rable's AST, so no part is an expansion node to find (#202).
            if has_backtick_substitution(value) {
                return true;
            }
            // Trust parsed parts; textual scan is only a fallback for synthetic
            // words. see docs/security-invariants.md#word-parts-trust
            if parts.is_empty() {
                has_shell_expansion_pattern(value)
            } else {
                parts.iter().any(has_expansions)
            }
        }
        NodeKind::Command {
            words, redirects, ..
        } => has_expansions_in_slices(words, redirects),
        NodeKind::Pipeline { commands, .. } => commands.iter().any(has_expansions),
        NodeKind::List { items } => items.iter().any(|item| has_expansions(&item.command)),
        NodeKind::Redirect { target, .. } => has_expansions(target),
        NodeKind::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            has_expansions(condition)
                || has_expansions(then_body)
                || else_body.as_deref().is_some_and(has_expansions)
        }
        NodeKind::Subshell { body, .. } | NodeKind::BraceGroup { body, .. } => has_expansions(body),
        NodeKind::HereDoc {
            content, quoted, ..
        } => !quoted && has_shell_expansion_pattern(content),
        _ => false,
    }
}

/// Returns `true` when `text` carries a backtick that bash would run as a
/// command substitution: one that is neither inside single quotes nor
/// backslash-escaped.
///
/// Rable lifts a bare `` `cmd` `` into a [`NodeKind::CommandSubstitution`]
/// part, but inside a double-quoted word it keeps the whole token as one
/// literal part — while bash still executes it. Walking the parts therefore
/// finds nothing, and this scan is the only signal (#202). Quote state is
/// tracked rather than scanning for a bare backtick so `'a `b` c'`, where the
/// backticks really are inert, is not falsely flagged.
#[must_use]
pub fn has_backtick_substitution(text: &str) -> bool {
    scan_interpreted(text, |c, _next, _in_double| c == '`')
}

/// Returns `true` when `text` carries a substitution bash resolves by *running*
/// a command: `$(...)`, an active backtick, or a `<(...)`/`>(...)` process
/// substitution.
///
/// Deliberately narrower than [`has_shell_expansion_pattern`], which also fires
/// on `$VAR` and `${VAR%%...}`. Callers that only need to know "does reading
/// this word execute anything" — a `case` subject is matched, never run (#193)
/// — must not treat a plain variable reference as dangerous.
#[must_use]
pub fn has_executing_substitution(text: &str) -> bool {
    scan_interpreted(text, |c, next, in_double| match c {
        '`' => true,
        '$' => next == Some('('),
        // `<(`/`>(` is literal text inside double quotes, unlike `$(`.
        '<' | '>' => !in_double && next == Some('('),
        _ => false,
    })
}

/// Run `is_hit` over the characters of `text` that bash would interpret,
/// skipping single-quoted and backslash-escaped ones, and report whether any
/// matched. `is_hit` receives the character, the raw character after it, and
/// whether the scan is inside double quotes.
fn scan_interpreted(text: &str, mut is_hit: impl FnMut(char, Option<char>, bool) -> bool) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            // A backslash is literal inside single quotes; everywhere else it
            // suppresses the next character.
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ if !in_single && is_hit(c, chars.peek().copied(), in_double) => return true,
            _ => {}
        }
    }
    false
}

/// Returns `true` when expanding `node` runs a command.
///
/// That means a `$(...)`/backtick command substitution or a `<(...)`/`>(...)`
/// process substitution, including one nested in an arithmetic expansion or in
/// the default/alternate text of a parameter expansion.
///
/// This is the "does it execute" half of [`has_expansions`]: an unset variable
/// or an unsupported parameter-expansion operator yields `false`, because
/// reading them runs nothing.
///
/// A word's raw text is scanned as well as its parts, because rable keeps a
/// double-quoted backtick as one literal (#202) and drops the inside of
/// `$((1+$(id)))` entirely — neither surfaces as a substitution part.
#[must_use]
pub fn word_executes_command(node: &Node) -> bool {
    match &node.kind {
        NodeKind::CommandSubstitution { .. } | NodeKind::ProcessSubstitution { .. } => true,
        NodeKind::Word { value, parts, .. } => {
            has_executing_substitution(value) || parts.iter().any(word_executes_command)
        }
        NodeKind::WordLiteral { value } | NodeKind::LocaleString { inner: value, .. } => {
            has_executing_substitution(value)
        }
        NodeKind::ParamExpansion { arg, .. } => {
            arg.as_deref().is_some_and(has_executing_substitution)
        }
        _ => false,
    }
}

/// Check if a string contains shell expansion patterns: `$(`, `` ` ``, `${`,
/// `$` + identifier, or `$` + a positional/special parameter (`$1`-`$9`, `$@`,
/// `$*`, `$#`, `$?`, `$$`, `$!`, `$-`).
///
/// Used for heredoc content and other string-level expansion detection where
/// structured AST nodes are not available.
#[must_use]
pub fn has_shell_expansion_pattern(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'`' {
            return true;
        }
        // rippy used a let-chain here; edition 2021 needs a nested if-let.
        if b == b'$' {
            if let Some(&next) = bytes.get(i + 1) {
                if next == b'('
                    || next == b'{'
                    || next == b'\''
                    || next == b'"'
                    || next.is_ascii_alphabetic()
                    || next == b'_'
                    || next.is_ascii_digit()
                    || matches!(next, b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-')
                {
                    return true;
                }
            }
        }
    }
    false
}
/// Redirect targets that discard or re-emit output and so cannot overwrite
/// anything worth guarding.
pub const SAFE_REDIRECT_TARGETS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr"];

/// Check if a redirect target is inherently safe (e.g., /dev/null).
#[must_use]
pub fn is_safe_redirect_target(target: &str) -> bool {
    SAFE_REDIRECT_TARGETS.contains(&target)
}

/// Whether the parsed tree is one plain simple command (no redirects, no word
/// expansions) — the only shape a whole-string allow rule may be trusted on.
///
/// A chain (`a && b`), pipeline (`a | b`), redirect (`a > f`), or command
/// substitution (`` a `b` ``) parses to a `List`/`Pipeline` or a `Command` that
/// carries redirects/expansions, none of which match here. Those fall through to
/// the AST walk so a trailing payload cannot ride along on a leading allow-ruled
/// command. see docs/security-invariants.md#string-rule-chokepoint
#[must_use]
pub fn is_single_plain_command(nodes: &[Node]) -> bool {
    let [node] = nodes else {
        return false;
    };
    matches!(
        &node.kind,
        NodeKind::Command { words, redirects, .. }
            if redirects.is_empty() && !words.iter().any(has_expansions)
    )
}

/// Returns `true` when a [`RedirectOp::FdDup`] target denotes a file
/// descriptor operation rather than a file write.
///
/// `&>`/`>&` are parsed as `FdDup`, but they mean two different things
/// depending on the target: a bare descriptor (`2>&1`, `>&2`) or a close
/// (`>&-`) is a real fd duplication, whereas a path (`&> out.log`) is a file
/// write and must be treated like `>`. Only the descriptor forms are matched
/// here: an optional leading `&`, then either `-` (close) or all-ASCII digits.
#[must_use]
pub fn is_fd_dup_target(target: &str) -> bool {
    let t = target.strip_prefix('&').unwrap_or(target);
    t == "-" || (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
}

/// Check if a node is a harmless fallback command (for `|| true` patterns).
#[must_use]
pub fn is_harmless_fallback(node: &Node) -> bool {
    let Some(name) = command_name(node) else {
        return false;
    };
    matches!(name, "true" | "false" | ":" | "echo" | "printf")
}

/// Extract text from a node, stripping quotes.
fn node_text(node: &Node) -> String {
    if let NodeKind::Word { value, .. } = &node.kind {
        strip_quotes(value)
    } else {
        String::new()
    }
}

/// Get the string value of a word node.
const fn word_value(node: &Node) -> Option<&str> {
    if let NodeKind::Word { value, .. } = &node.kind {
        Some(value.as_str())
    } else {
        None
    }
}
/// Returns `true` when a node is a safe heredoc data-passing idiom:
/// a single `SIMPLE_SAFE` command whose only redirects are quoted heredocs,
/// with no word-level expansions.
///
/// Example: `cat <<'EOF' ... EOF` — `cat` is safe, heredoc is quoted,
/// no pipes, no lists.
///
/// Reliability note: the structural guarantees here assume rable produces
/// a faithful AST for heredocs inside `$(...)`. That held unreliably before
/// rable 0.1.14 (see rable issue #26) — an unmatched `(` in a heredoc body
/// could corrupt paren tracking and drop the `HereDoc` node. Pin `rable >=
/// 0.1.14` when touching this helper.
///
/// Scope note: the checks here are intentionally not tightened further
/// (e.g., restricting to literally `cat` or `words.len() == 1`). `SIMPLE_SAFE`
/// is a read-only-viewer allowlist (`cat`, `head`, `grep`, `xxd`, …) with no
/// command-execution primitives, so a non-`cat` entry with flags and a
/// quoted heredoc is still safe data-passing. The existing conditions
/// (`SIMPLE_SAFE` + all-redirects-quoted-heredocs + no word-expansions) are
/// already structurally tight; the rable 0.1.14 fix makes them *reliable*.
#[must_use]
pub fn is_safe_heredoc_substitution(command: &Node) -> bool {
    let NodeKind::Command {
        words, redirects, ..
    } = &command.kind
    else {
        return false;
    };
    let Some(name) = command_name_from_words(words) else {
        return false;
    };
    if !allowlists::is_simple_safe(name) {
        return false;
    }
    if redirects.is_empty() {
        return false;
    }
    let all_quoted_heredocs = redirects
        .iter()
        .all(|r| matches!(&r.kind, NodeKind::HereDoc { quoted, .. } if *quoted));
    if !all_quoted_heredocs {
        return false;
    }
    !words.iter().any(has_expansions)
}

#[path = "ast_quote.rs"]
pub(crate) mod quote;

#[path = "ast_env.rs"]
pub(crate) mod env;

pub(crate) use env::{
    append_assignment_name, is_dangerous_env_name, literal_assignment, strip_env_prefix,
};
pub(crate) use quote::strip_quotes;
