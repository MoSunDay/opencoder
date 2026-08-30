//! Expansion machinery of the resolver: word-kind resolution, part
//! combination, arithmetic evaluation and brace expansion.
//! Ported from rippy `src/resolve.rs` (MIT, https://github.com/mpecan/rippy).

use rable::{Node, NodeKind};

use crate::ast;

use super::{literal_if_inert, resolve_param_expansion, resolve_word, strip_outer_quotes, VarLookup, WordResolution};

pub(crate) fn resolve_word_kind(kind: &NodeKind, vars: &dyn VarLookup) -> WordResolution {
    match kind {
        NodeKind::Word { value, parts, .. } => resolve_word_node(value, parts, vars),
        // A double-quoted backtick reaches us as plain literal text (#202); it
        // must never resolve to itself as inert data.
        NodeKind::WordLiteral { value } if ast::has_backtick_substitution(value) => {
            WordResolution::Unresolvable {
                reason: "command substitution requires execution".to_string(),
            }
        }
        NodeKind::WordLiteral { value } => WordResolution::Literal(value.clone()),
        NodeKind::AnsiCQuote { decoded, .. } => WordResolution::Literal(decoded.clone()),
        NodeKind::LocaleString { inner, .. } => literal_if_inert(inner, "$\"...\" locale string"),
        NodeKind::ParamExpansion { param, op, arg } => {
            resolve_param_expansion(param, op.as_deref(), arg.as_deref(), vars)
        }
        NodeKind::ParamLength { param } => WordResolution::Unresolvable {
            reason: format!("${{#{param}}} length expansion is not supported"),
        },
        NodeKind::ParamIndirect { param, .. } => WordResolution::Unresolvable {
            reason: format!("${{!{param}}} indirect expansion is not supported"),
        },
        NodeKind::ArithmeticExpansion { expression } => resolve_arithmetic(expression.as_deref()),
        NodeKind::BraceExpansion { content } => expand_brace(content).map_or_else(
            || WordResolution::Unresolvable {
                reason: format!("brace expansion {content} could not be expanded"),
            },
            WordResolution::Multiple,
        ),
        NodeKind::CommandSubstitution { command, .. }
            if ast::is_safe_heredoc_substitution(command) =>
        {
            resolve_safe_heredoc_content(command)
        }
        NodeKind::CommandSubstitution { .. } => WordResolution::Unresolvable {
            reason: "command substitution requires execution".to_string(),
        },
        NodeKind::ProcessSubstitution { .. } => WordResolution::Unresolvable {
            reason: "process substitution requires execution".to_string(),
        },
        _ => WordResolution::Unresolvable {
            reason: "non-word node".to_string(),
        },
    }
}

/// Extract the concatenated heredoc content from a safe heredoc command.
/// Caller must ensure `is_safe_heredoc_substitution(command)` is true.
fn resolve_safe_heredoc_content(command: &Node) -> WordResolution {
    let NodeKind::Command { redirects, .. } = &command.kind else {
        return WordResolution::Unresolvable {
            reason: "expected Command node".to_string(),
        };
    };
    let mut content = String::new();
    for redir in redirects {
        if let NodeKind::HereDoc {
            content: body,
            quoted,
            ..
        } = &redir.kind
        {
            if !quoted {
                return WordResolution::Unresolvable {
                    reason: "unquoted heredoc".to_string(),
                };
            }
            content.push_str(body);
        }
    }
    WordResolution::Literal(content)
}

fn resolve_word_node(value: &str, parts: &[Node], vars: &dyn VarLookup) -> WordResolution {
    if parts.is_empty() {
        return WordResolution::Literal(strip_outer_quotes(value));
    }
    let mut resolved_parts: Vec<WordResolution> = Vec::with_capacity(parts.len());
    let mut dynamic = false;
    for part in parts {
        match resolve_word(part, vars) {
            // Unresolvable is the most conservative outcome — it always forces
            // Ask — so it wins over a sibling DynamicKnown part.
            WordResolution::Unresolvable { reason } => {
                return WordResolution::Unresolvable { reason };
            }
            WordResolution::DynamicKnown => dynamic = true,
            r => resolved_parts.push(r),
        }
    }
    if dynamic {
        return WordResolution::DynamicKnown;
    }
    combine_parts(&resolved_parts)
}

/// Combine resolved parts. Mixing `Multiple` parts with literals produces a
/// cartesian expansion (`file.{a,b}` → `[file.a, file.b]`).
///
/// Refuses patterns whose cartesian product would exceed `MAX_BRACE_EXPANSION`
/// items, returning `Unresolvable` so the caller falls back to Ask. This
/// prevents `{1..32}{1..32}{1..32}` (32k items) from exhausting memory.
pub(crate) fn combine_parts(parts: &[WordResolution]) -> WordResolution {
    let mut variants: Vec<String> = vec![String::new()];
    for part in parts {
        match part {
            WordResolution::Literal(s) => {
                for v in &mut variants {
                    v.push_str(s);
                }
            }
            WordResolution::Multiple(items) => {
                let projected = variants.len().saturating_mul(items.len());
                if projected > MAX_BRACE_EXPANSION {
                    return WordResolution::Unresolvable {
                        reason: format!(
                            "brace expansion would produce {projected} items \
                             (cap: {MAX_BRACE_EXPANSION})"
                        ),
                    };
                }
                let mut next = Vec::with_capacity(projected);
                for v in &variants {
                    for item in items {
                        let mut combined = v.clone();
                        combined.push_str(item);
                        next.push(combined);
                    }
                }
                variants = next;
            }
            // Filtered out by `resolve_word_node` before this is called, so not
            // reached today — but a security hook must not be one refactor away
            // from a panic, so fail closed (Ask) instead of `unreachable!`.
            WordResolution::Unresolvable { .. } | WordResolution::DynamicKnown => {
                return WordResolution::Unresolvable {
                    reason: "unexpected resolution state".to_string(),
                };
            }
        }
    }
    if variants.len() == 1 {
        WordResolution::Literal(variants.into_iter().next().unwrap_or_default())
    } else {
        WordResolution::Multiple(variants)
    }
}

/// The default/alternate text of `${VAR:-x}` / `${VAR-x}` / `${VAR:+x}` and the
/// inner of a `$"..."` locale string are RE-EXPANDED by bash at runtime
/// (`$(...)`, backticks, and `$var` inside them all run). Return such text as an
/// inert `Literal` only when it carries no shell-expansion pattern and no
/// process substitution; otherwise `Unresolvable` so the caller falls back to
/// Ask — never Allow attacker-derived text as a harmless literal. See #156.
/// (A SET variable's value, by contrast, is NOT re-expanded by bash, so the
fn resolve_arithmetic(expression: Option<&Node>) -> WordResolution {
    expression.and_then(eval_arithmetic).map_or_else(
        || WordResolution::Unresolvable {
            reason: "arithmetic expression could not be evaluated".to_string(),
        },
        |n| WordResolution::Literal(n.to_string()),
    )
}

/// Evaluate an arithmetic expression node if all leaves are constants.
fn eval_arithmetic(expr: &Node) -> Option<i64> {
    match &expr.kind {
        NodeKind::ArithNumber { value } => parse_arith_number(value),
        NodeKind::ArithBinaryOp { op, left, right } => {
            let l = eval_arithmetic(left)?;
            let r = eval_arithmetic(right)?;
            apply_binary(op, l, r)
        }
        NodeKind::ArithUnaryOp { op, operand } => {
            let v = eval_arithmetic(operand)?;
            apply_unary(op, v)
        }
        _ => None,
    }
}

fn parse_arith_number(value: &str) -> Option<i64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return i64::from_str_radix(hex, 16).ok();
    }
    if value.starts_with('0') && value.len() > 1 && !value.contains(|c: char| !c.is_ascii_digit()) {
        return i64::from_str_radix(&value[1..], 8).ok();
    }
    value.parse::<i64>().ok()
}

fn apply_binary(op: &str, l: i64, r: i64) -> Option<i64> {
    match op {
        "+" => l.checked_add(r),
        "-" => l.checked_sub(r),
        "*" => l.checked_mul(r),
        "/" if r != 0 => l.checked_div(r),
        "%" if r != 0 => l.checked_rem(r),
        "**" => {
            let exp = u32::try_from(r).ok()?;
            l.checked_pow(exp)
        }
        "<<" => {
            let shift = u32::try_from(r).ok()?;
            l.checked_shl(shift)
        }
        ">>" => {
            let shift = u32::try_from(r).ok()?;
            l.checked_shr(shift)
        }
        "&" => Some(l & r),
        "|" => Some(l | r),
        "^" => Some(l ^ r),
        _ => None,
    }
}

fn apply_unary(op: &str, v: i64) -> Option<i64> {
    match op {
        "+" => Some(v),
        "-" => v.checked_neg(),
        "~" => Some(!v),
        "!" => Some(i64::from(v == 0)),
        _ => None,
    }
}

/// Maximum number of items a single brace expansion may produce.
///
/// Bash has no built-in cap, but we refuse to materialize anything larger
/// to prevent `{1..1000000000}` from exhausting memory. Patterns that would
/// exceed this cap are treated as `Unresolvable` (caller falls back to Ask).
const MAX_BRACE_EXPANSION: usize = 1024;

/// Expand a brace pattern like `{a,b,c}` or `{1..10}`.
///
/// Returns `None` if the pattern is malformed, contains nested braces,
/// or would produce more than `MAX_BRACE_EXPANSION` items.
fn expand_brace(content: &str) -> Option<Vec<String>> {
    let bytes = content.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'{' || bytes[bytes.len() - 1] != b'}' {
        return None;
    }
    let inner = &content[1..content.len() - 1];
    if inner.contains('{') || inner.contains('}') {
        return None; // nested braces — defer to follow-up
    }
    if let Some(range) = parse_range(inner) {
        return if range.len() <= MAX_BRACE_EXPANSION {
            Some(range)
        } else {
            None
        };
    }
    let items: Vec<String> = inner.split(',').map(str::to_string).collect();
    if items.len() < 2 || items.len() > MAX_BRACE_EXPANSION {
        return None;
    }
    Some(items)
}

fn parse_range(inner: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = inner.splitn(3, "..").collect();
    if parts.len() < 2 {
        return None;
    }
    if let (Ok(start), Ok(end)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
        return numeric_range(start, end);
    }
    if parts[0].len() == 1 && parts[1].len() == 1 {
        let start = parts[0].chars().next()?;
        let end = parts[1].chars().next()?;
        if start.is_ascii() && end.is_ascii() {
            return Some(char_range(start, end));
        }
    }
    None
}

/// Build a numeric range, refusing patterns that would exceed
/// `MAX_BRACE_EXPANSION` items (returns `None` so the caller falls back to Ask).
fn numeric_range(start: i64, end: i64) -> Option<Vec<String>> {
    let span = (end - start).unsigned_abs();
    if span >= MAX_BRACE_EXPANSION as u64 {
        return None;
    }
    Some(if start <= end {
        (start..=end).map(|n| n.to_string()).collect()
    } else {
        (end..=start).rev().map(|n| n.to_string()).collect()
    })
}

fn char_range(start: char, end: char) -> Vec<String> {
    // ASCII-bounded (max 128 items), well under MAX_BRACE_EXPANSION.
    let s = start as u8;
    let e = end as u8;
    if s <= e {
        (s..=e).map(|b| (b as char).to_string()).collect()
    } else {
        (e..=s).rev().map(|b| (b as char).to_string()).collect()
    }
}

