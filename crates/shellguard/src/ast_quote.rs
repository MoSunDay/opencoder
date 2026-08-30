//! Quoting machinery for word-value dequoting.
//!
//! Ported from rippy `src/ast.rs` (MIT, https://github.com/mpecan/rippy): rable
//! keeps the quote characters in a word value, so tokens must be dequoted
//! wherever a quote appears before any handler compares them.

use std::iter::Peekable;
use std::str::Chars;

/// The quoting context a token scan is in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quoting {
    Bare,
    Single,
    Double,
}

/// Resolve a token to the literal argument text the shell hands the command.
///
/// rable keeps the quote characters in the word value, so matching a handler's
/// flag against the raw token matches the attacker's *spelling* rather than the
/// value the command receives: `--to-com'mand'` and `--to-command""` are both
/// literally `--to-command` (#198). Quoting is therefore removed wherever it
/// appears in the token, not just when it wraps the whole of it.
pub(crate) fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if has_embedded_dollar_quote(s) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut state = Quoting::Bare;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        state = match state {
            Quoting::Bare => scan_bare(c, &mut chars, &mut out),
            Quoting::Single if c == '\'' => Quoting::Bare,
            Quoting::Single => {
                out.push(c);
                Quoting::Single
            }
            Quoting::Double => scan_double(c, &mut chars, &mut out),
        };
    }
    // An unbalanced quote means rable's tokenizer and the shell disagree about
    // where the word ends, so the dequoted text is not the runtime value of
    // anything; keep the raw token rather than invent one.
    if state == Quoting::Bare {
        out
    } else {
        s.to_owned()
    }
}

/// Returns `true` when a `$'…'` or `$"…"` starts anywhere but at the front of
/// the token.
///
/// Such a token is left raw: guards downstream re-scan the resolved text for
/// `$` to decide whether a value is statically known (a redirect target such as
/// `/tmp/foo$"x"` must keep asking), and dequoting would erase the sigil they
/// look for. Nothing is lost — a word carrying either form parses to an
/// `AnsiCQuote`/`LocaleString` part, so the expansion stage has already asked
/// before any handler sees it.
fn has_embedded_dollar_quote(s: &str) -> bool {
    s.match_indices('$')
        .any(|(i, _)| i > 0 && matches!(s[i + 1..].chars().next(), Some('\'' | '"')))
}

/// One unquoted character: opens a quote, resolves a backslash escape, drops the
/// `$` of a `$'…'` / `$"…"` literal, or contributes itself.
fn scan_bare(c: char, chars: &mut Peekable<Chars<'_>>, out: &mut String) -> Quoting {
    match c {
        '\'' => Quoting::Single,
        '"' => Quoting::Double,
        '\\' => {
            out.extend(chars.next());
            Quoting::Bare
        }
        '$' if matches!(chars.peek(), Some('\'' | '"')) => Quoting::Bare,
        _ => {
            out.push(c);
            Quoting::Bare
        }
    }
}

/// One character inside double quotes, where only `"`, `\`, `$` and a backtick
/// can be backslash-escaped.
fn scan_double(c: char, chars: &mut Peekable<Chars<'_>>, out: &mut String) -> Quoting {
    match c {
        '"' => Quoting::Bare,
        '\\' if matches!(chars.peek(), Some('"' | '\\' | '$' | '`')) => {
            out.extend(chars.next());
            Quoting::Double
        }
        _ => {
            out.push(c);
            Quoting::Double
        }
    }
}
