//! Static expansion resolution: turns `$HOME`, `$'hello'`, `$((1+1))`, `{a,b}`
//! into concrete strings using rable's AST and the host environment.
//!
//! The resolved command is then re-classified through the full analyzer
//! pipeline, so the variable's *content* (not its name) determines the verdict.

use rable::{Node, NodeKind};

use crate::ast;

/// Result of resolving a single word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WordResolution {
    /// All parts resolved to a single literal string.
    Literal(String),
    /// Brace expansion produced multiple words (changes argument count).
    Multiple(Vec<String>),
    /// The word references a variable that is known to be *set* but whose
    /// value is not statically known (a for/select loop variable or a shell
    /// status/special var such as `$?`). We deliberately do NOT fabricate a
    /// value for it: substituting a placeholder and re-analyzing could hide an
    /// injected dangerous flag and flip a handler command from Ask to Allow.
    /// Callers gate on this outcome instead (see [`ResolvedArgs`]).
    DynamicKnown,
    /// At least one part is unresolvable.
    Unresolvable {
        /// Human-readable explanation of why resolution failed.
        reason: String,
    },
}

/// The static resolution state of a variable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarState {
    /// The variable is unset.
    Unset,
    /// The variable is set to a statically-known literal value.
    Value(String),
    /// The variable is known to be set, but its value is not statically known
    /// (loop-iteration variable, shell status/special variable, ...).
    DynamicSet,
}

/// A variable binding local to the command currently being analyzed.
///
/// Bindings are collected on the analyzer's `locals` stack and consulted by
/// [`ScopedLookup`] with a strict lexical-scope (checkpoint/truncate)
/// discipline, so they never satisfy a `$VAR` outside the scope that bound them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalBinding {
    /// Value fully known statically (literal `VAR=val` assignment / prefix).
    /// Substitutes its real value exactly like an env var and is re-analyzed —
    /// no injection surface beyond today's env path.
    Literal(String),
    /// Known to be set, value unknown (for/select loop variable). Resolves to
    /// [`WordResolution::DynamicKnown`], never a fabricated string.
    Dynamic,
}

/// Outcome of resolving a full argument list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedArgs {
    /// Resolved argument list, or `None` if any word was unresolvable
    /// or a `DynamicKnown` value landed in argument position.
    pub args: Option<Vec<String>>,
    /// True if the first word (command position) contains a parameter expansion.
    /// Forces Ask even when resolution succeeds — `$cmd args` is always dangerous.
    pub command_position_dynamic: bool,
    /// True if a *non-first* word resolved to [`WordResolution::DynamicKnown`]
    /// — a set-but-unknown value in argument position. The command may be
    /// allowed only if `failure_reason` is `None` (no other word was
    /// unresolvable) *and* its literal name is dynamic-arg-safe (a pure reader,
    /// safe regardless of argument values); every other command stays Ask.
    pub arg_position_dynamic: bool,
    /// Reason from the first unresolvable word (for Ask diagnostics). Set
    /// independently of `arg_position_dynamic`: a word list may contain both a
    /// dynamic-known argument and a later unresolvable substitution, and the
    /// latter must still dominate.
    pub failure_reason: Option<String>,
}

/// Trait for looking up variable values. Allows test injection without
/// touching the real process environment.
pub trait VarLookup: Send + Sync {
    /// Returns `Some(value)` if the variable is set, `None` if unset.
    fn lookup(&self, name: &str) -> Option<String>;

    /// Returns the static resolution [`VarState`] of a variable name.
    ///
    /// The default implementation maps `lookup` onto `Value`/`Unset`, so
    /// existing implementors (`EnvLookup`, test mocks) work unchanged.
    /// [`ScopedLookup`] overrides this to surface local and status-var
    /// bindings.
    fn state(&self, name: &str) -> VarState {
        self.lookup(name).map_or(VarState::Unset, VarState::Value)
    }
}

/// Returns `true` if `name` is a shell status/special variable.
///
/// Such variables are always considered *set* with a dynamic value: `$?`, `$$`,
/// `$#`, `$!`, `$-`, `$*`, `$@`, numbered positional parameters, and named
/// specials such as `PIPESTATUS`, `RANDOM`, `SECONDS`, ... The base name is
/// matched *before* any `[` array subscript, so `${PIPESTATUS[0]}` is
/// recognized too.
#[must_use]
pub(crate) fn is_status_var(name: &str) -> bool {
    let base = name.split('[').next().unwrap_or(name);
    if base.is_empty() {
        return false;
    }
    if base.bytes().all(|b| b.is_ascii_digit()) {
        return true; // positional parameters ($0, $1, $2, ...)
    }
    matches!(
        base,
        "?" | "$"
            | "#"
            | "!"
            | "-"
            | "*"
            | "@"
            | "PIPESTATUS"
            | "RANDOM"
            | "SECONDS"
            | "LINENO"
            | "BASHPID"
            | "PPID"
            | "UID"
            | "EUID"
    )
}

/// A [`VarLookup`] that overlays command-local bindings and shell status
/// variables on top of an inner lookup (usually the process environment).
///
/// `state` consults `locals` (most-recent binding wins) and status-var names
/// first, then falls back to `inner`. Literal locals surface their value;
/// dynamic locals and status vars surface [`VarState::DynamicSet`].
pub(crate) struct ScopedLookup<'a> {
    locals: &'a [(String, LocalBinding)],
    inner: &'a dyn VarLookup,
}

impl<'a> ScopedLookup<'a> {
    /// Wrap `inner` with the given command-local bindings.
    #[must_use]
    pub(crate) const fn new(
        locals: &'a [(String, LocalBinding)],
        inner: &'a dyn VarLookup,
    ) -> Self {
        Self { locals, inner }
    }

    fn local(&self, name: &str) -> Option<&LocalBinding> {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b)
    }
}

impl VarLookup for ScopedLookup<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match self.local(name) {
            Some(LocalBinding::Literal(v)) => Some(v.clone()),
            Some(LocalBinding::Dynamic) => None,
            None => self.inner.lookup(name),
        }
    }

    fn state(&self, name: &str) -> VarState {
        match self.local(name) {
            Some(LocalBinding::Literal(v)) => VarState::Value(v.clone()),
            Some(LocalBinding::Dynamic) => VarState::DynamicSet,
            None if is_status_var(name) => VarState::DynamicSet,
            None => self.inner.state(name),
        }
    }
}

/// Production env-based lookup. Reads `std::env::var` for any variable name.
///
/// No allowlist — the resolved value is re-classified through the full
/// analyzer pipeline, so the variable's content (not its name) determines
/// the verdict.
pub struct EnvLookup;

impl VarLookup for EnvLookup {
    fn lookup(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Attempt to resolve a single word node into literal text (or multiple words).
pub(crate) fn resolve_word(node: &Node, vars: &dyn VarLookup) -> WordResolution {
    expand::resolve_word_kind(&node.kind, vars)
}


fn literal_if_inert(text: &str, what: &str) -> WordResolution {
    if ast::has_shell_expansion_pattern(text) || has_process_substitution(text) {
        WordResolution::Unresolvable {
            reason: format!("{what} contains a shell expansion requiring execution"),
        }
    } else {
        WordResolution::Literal(text.to_string())
    }
}

/// Detect bash process substitution `<(...)` / `>(...)`. `has_shell_expansion_pattern`
/// keys on `$`/backtick and misses these, yet bash runs the inner command when it
/// expands the default/alternate/locale text they are embedded in. See #156.
pub(crate) fn has_process_substitution(text: &str) -> bool {
    text.as_bytes()
        .windows(2)
        .any(|w| (w[0] == b'<' || w[0] == b'>') && w[1] == b'(')
}

fn resolve_param_expansion(
    param: &str,
    op: Option<&str>,
    arg: Option<&str>,
    vars: &dyn VarLookup,
) -> WordResolution {
    let state = vars.state(param);
    match (op, arg, &state) {
        // ${VAR}/${VAR:-def} on a set value returns that value.
        (None | Some(":-" | "-"), _, VarState::Value(v)) => WordResolution::Literal(v.clone()),
        (None | Some(":-" | "-"), _, VarState::DynamicSet) => WordResolution::DynamicKnown,
        (None, _, VarState::Unset) => WordResolution::Unresolvable {
            reason: format!("${param} is not set"),
        },
        (Some(":-" | "-"), Some(default), VarState::Unset) => {
            literal_if_inert(default, "${...:-} default")
        }
        // `:+` alternate comes from source text, so DynamicSet still yields it.
        (Some(":+"), Some(value), VarState::Value(_) | VarState::DynamicSet) => {
            literal_if_inert(value, "${...:+} alternate")
        }
        (Some(":+"), _, VarState::Unset) => WordResolution::Literal(String::new()),
        (Some(op), _, _) => WordResolution::Unresolvable {
            reason: format!("${{{param}{op}...}} operator not supported"),
        },
    }
}

pub(crate) fn strip_outer_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

/// Resolve all words in a command's `words` slice.
///
/// Returns the resolved arg list (or `None` if any word is unresolvable),
/// plus a flag indicating whether the first word (command position) contains
/// a `ParamExpansion` — which forces Ask even when resolution succeeds.
#[must_use]
pub(crate) fn resolve_command_args(words: &[Node], vars: &dyn VarLookup) -> ResolvedArgs {
    let command_position_dynamic = words.first().is_some_and(word_has_param_expansion);
    let mut resolved: Vec<String> = Vec::with_capacity(words.len());
    let mut failure_reason: Option<String> = None;
    let mut arg_position_dynamic = false;
    let mut all_ok = true;
    for (i, word) in words.iter().enumerate() {
        match resolve_word(word, vars) {
            WordResolution::Literal(s) => resolved.push(s),
            WordResolution::Multiple(items) => resolved.extend(items),
            // Set-but-unknown: flag arg-position, never fabricate, and keep
            // scanning. see docs/security-invariants.md#dynamic-arg
            WordResolution::DynamicKnown => {
                if i > 0 {
                    arg_position_dynamic = true;
                }
                all_ok = false;
            }
            WordResolution::Unresolvable { reason } => {
                if failure_reason.is_none() {
                    failure_reason = Some(reason);
                }
                all_ok = false;
            }
        }
    }
    ResolvedArgs {
        args: if all_ok { Some(resolved) } else { None },
        command_position_dynamic,
        arg_position_dynamic,
        failure_reason,
    }
}

fn word_has_param_expansion(node: &Node) -> bool {
    match &node.kind {
        NodeKind::ParamExpansion { .. } | NodeKind::ParamIndirect { .. } => true,
        NodeKind::Word { parts, .. } => parts.iter().any(word_has_param_expansion),
        _ => false,
    }
}

/// Quote an argument for inclusion in a re-parsable shell command.
///
/// If the argument contains shell metacharacters or whitespace, it is
/// single-quoted with internal single quotes escaped as `'\''`.
#[must_use]
pub(crate) fn shell_join_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.bytes().all(is_safe_unquoted) {
        return arg.to_string();
    }
    let escaped = arg.replace('\'', r"'\''");
    format!("'{escaped}'")
}

const fn is_safe_unquoted(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'/' | b'.' | b','
    )
}

/// Join resolved args into a single shell-safe command string.
#[must_use]
pub(crate) fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| shell_join_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[path = "resolve_expand.rs"]
pub(crate) mod expand;
