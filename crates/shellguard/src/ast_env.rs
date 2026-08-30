//! Env-prefix stripping, literal assignment binding and the
//! code-influencing-variable gate.
//!
//! Ported from rippy `src/ast.rs` (MIT, https://github.com/mpecan/rippy).

use rable::{Node, NodeKind};

use super::strip_quotes;
use super::{has_backtick_substitution, has_expansions};

/// Drop a leading `NAME=VALUE` env prefix, returning the rest of the command
/// verbatim (words, pipes, `&&`/`||`/`;` chains, and redirects all preserved).
///
/// This lets the config/permission string matchers see the real command name
/// (`cargo`) rather than the assignment token (`INSTA_UPDATE=always`), while the
/// result stays byte-for-byte identical to the same command written without the
/// prefix — so an env-prefixed command is treated exactly like its bare form and
/// no new redirect/pipeline path is introduced.
///
/// The prefix is located on the *leftmost* simple command, so pipelines and
/// lists are handled too: `RUST_LOG=debug cargo test | grep foo` becomes
/// `cargo test | grep foo`, and `A=1 cargo test && cargo build` becomes
/// `cargo test && cargo build`. The tail is kept by slicing the original string
/// from the first word's own source span, never a hand-rolled tokenizer.
///
/// Returns `None` (caller keeps the original string) when:
/// - the input is not a single top-level statement, or its leftmost node is not
///   a simple command carrying at least one assignment and one word;
/// - any assignment value contains a shell expansion — a deliberate coupling, so
///   an ALLOW rule cannot mask `FOO=$(rm -rf /) cargo test`; those are forced to
///   Ask by the analyzer's assignment-expansion guard instead;
/// - any assignment sets a code-influencing variable (see
///   [`is_dangerous_env_name`]), so a literal `LD_PRELOAD=./evil.so cargo test`
///   is not masked by a bare-command allow rule and instead reaches the analyzer.
#[must_use]
pub(crate) fn strip_env_prefix(command: &str, nodes: &[Node]) -> Option<String> {
    let [node] = nodes else {
        return None;
    };
    let (assignments, words) = leftmost_simple_command(node)?;
    if assignments.is_empty() || words.is_empty() {
        return None;
    }
    if assignments.iter().any(has_expansions) {
        return None;
    }
    if assignments
        .iter()
        .filter_map(|a| assignment_name(a, command))
        .any(is_dangerous_env_name)
    {
        return None;
    }
    let char_start = words.first()?.span.start;
    let byte_start = command.char_indices().nth(char_start).map(|(i, _)| i)?;
    command.get(byte_start..).map(str::to_owned)
}

/// Find the leftmost simple command, descending through the first branch of a
/// pipeline or list, and return its `(assignments, words)`.
fn leftmost_simple_command(node: &Node) -> Option<(&[Node], &[Node])> {
    match &node.kind {
        NodeKind::Command {
            assignments, words, ..
        } => Some((assignments, words)),
        NodeKind::Pipeline { commands, .. } => commands.first().and_then(leftmost_simple_command),
        NodeKind::List { items } => items
            .first()
            .and_then(|item| leftmost_simple_command(&item.command)),
        _ => None,
    }
}

/// Extract the `(name, value)` of a *literal* `NAME=VALUE` assignment node —
/// i.e. one whose value contains no shell expansion.
///
/// Reads the name and value directly from the assignment `Word`'s `value`
/// (`"NAME=VALUE"`), so no source string needs threading into the deep walk.
/// Returns `None` when the node is not a word, the value contains an expansion
/// (command substitution, parameter expansion, ...), or the name is empty.
///
/// A `NAME+=VALUE` compound assignment is deliberately *not* bound: bash
/// concatenates `VALUE` onto the variable's existing value, so binding it to the
/// right-hand side alone would be wrong. We return `None` (the variable stays
/// unresolved and the referencing command falls back to Ask) rather than
/// fabricate a truncated value.
///
/// The value has any outer quotes stripped, matching how the analyzer treats
/// argument words, so `FOO='a b'` yields `("FOO", "a b")`.
///
/// The expansion check reads the raw text as well as the parsed parts: a
/// quoted-backtick value (`` x="`cmd`" ``) has no expansion part to find, so
/// the parts walk alone would bind attacker-chosen output as a literal (#202).
#[must_use]
pub(crate) fn literal_assignment(assignment: &Node) -> Option<(String, String)> {
    let NodeKind::Word { value, parts, .. } = &assignment.kind else {
        return None;
    };
    if has_backtick_substitution(value) || parts.iter().any(has_expansions) {
        return None;
    }
    let (name, val) = value.split_once('=')?;
    // `NAME+=VALUE` appends to the prior value we cannot reconstruct; refuse.
    if name.ends_with('+') {
        return None;
    }
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), strip_quotes(val)))
}

/// Return the variable name of a `NAME+=VALUE` compound (append) assignment
/// node, or `None` if the node is not an append assignment (`NAME=VALUE`) or
/// not a word.
///
/// An append combines the variable's *prior* value (which may not be statically
/// known, or may live in a shadowed binding) with the right-hand side, so no
/// concrete value can be bound soundly. The analyzer uses this to shadow the
/// variable as set-but-unknown — keeping safe-list commands allowed while
/// forcing handlers to Ask, and never resolving a stale prior literal.
#[must_use]
pub(crate) fn append_assignment_name(assignment: &Node) -> Option<String> {
    let NodeKind::Word { value, .. } = &assignment.kind else {
        return None;
    };
    let (name, _) = value.split_once('=')?;
    // `strip_suffix('+')` yields `None` unless the name ends with `+`, so this
    // matches `NAME+=...` only, never a plain `NAME=...`.
    let base = name.strip_suffix('+')?;
    if base.is_empty() {
        return None;
    }
    Some(base.to_string())
}

/// Extract the variable name of a `NAME=VALUE` (or `NAME+=VALUE`) assignment node.
fn assignment_name<'a>(assignment: &Node, source: &'a str) -> Option<&'a str> {
    let text = assignment.source_text(source);
    let (name, _) = text.split_once('=')?;
    Some(name.strip_suffix('+').unwrap_or(name))
}

/// Environment variable names whose values can change how a following command
/// loads or resolves code, letting a *literal* assignment turn an otherwise-safe
/// command into arbitrary code execution (e.g. `LD_PRELOAD`, `BASH_ENV`,
/// `GIT_SSH_COMMAND`). The analyzer's assignment-name gate Asks on any command
/// carrying such a prefix, and [`strip_env_prefix`] refuses to strip it so a
/// string-layer allow rule cannot mask it either.
///
/// The `GIT_CONFIG`/`BASH_FUNC_` prefix matches are deliberately broad — each
/// covers a whole injection family in one check, mirroring the `LD_`/`DYLD_`
/// style. See docs/security-invariants.md#dangerous-env-name for the rationale.
#[must_use]
pub(crate) fn is_dangerous_env_name(name: &str) -> bool {
    // Dynamic-linker families: Linux `LD_*` (LD_PRELOAD, LD_LIBRARY_PATH,
    // LD_AUDIT, ...) and macOS `DYLD_*` (DYLD_INSERT_LIBRARIES, ...).
    if name.starts_with("LD_") || name.starts_with("DYLD_") {
        return true;
    }
    // GIT_CONFIG* = env-based git-config injection; BASH_FUNC_* = exported
    // function injection. see docs/security-invariants.md#dangerous-env-name
    if name.starts_with("GIT_CONFIG") || name.starts_with("BASH_FUNC_") {
        return true;
    }
    // ANSIBLE_*_PLUGINS point ansible at an attacker-chosen directory it then
    // imports Python from — the env route to what `ansible-doc -M` does (#185).
    if name.starts_with("ANSIBLE_") && name.ends_with("_PLUGINS") {
        return true;
    }
    matches!(
        name,
        "BASH_ENV"
            | "ENV"
            | "SHELLOPTS"
            | "BASHOPTS"
            | "IFS"
            | "PS4"
            | "GIT_SSH"
            | "GIT_SSH_COMMAND"
            | "GIT_EXTERNAL_DIFF"
            | "GIT_PAGER"
            | "PAGER"
            | "EDITOR"
            | "VISUAL"
            | "PERL5OPT"
            | "PERL5LIB"
            | "PYTHONSTARTUP"
            | "PYTHONPATH"
            | "NODE_OPTIONS"
            | "RUBYOPT"
            | "ANSIBLE_CONFIG"
            | "ANSIBLE_LIBRARY"
            | "ANSIBLE_MODULE_UTILS"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_dangerous_env_name_flags_git_config_and_bash_func_families() {
        for name in [
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_PARAMETERS",
            "BASH_FUNC_foo%%",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "GIT_SSH_COMMAND",
        ] {
            assert!(is_dangerous_env_name(name), "{name} should be dangerous");
        }
    }

    #[test]
    fn is_dangerous_env_name_allows_ordinary_names() {
        for name in ["FOO", "PATH", "HOME", "NODE_ENV", "CI", "RUST_LOG"] {
            assert!(!is_dangerous_env_name(name), "{name} should be safe");
        }
    }
}
