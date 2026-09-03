//! Structured clean-view transformation for copy mode.
//!
//! Instead of guessing decoration from flattened text (the old heuristics
//! mis-killed box-drawing characters inside code and mis-stripped gutters on
//! space-led prose), the cleaner works on the rendered [`Line`] *span
//! structure*: the exact decoration shapes are declared once as constants in
//! the renderers ([`crate::markdown`], [`crate::chat`]) and matched here, so
//! any shape drift is shared at compile time. Body indentation gutters are
//! independent all-space spans whose width carries the slot exactly — no
//! 4-vs-2-space guessing — and rows that merely *contain* decoration glyphs
//! (e.g. `---` YAML frontmatter inside a fenced block, which carries a `│ `
//! prefix span) are content, not chrome, and survive verbatim.

use ratatui::text::{Line, Span};

use crate::chat::{
    GROUP_ROW_CLOSED_PREFIX, GROUP_ROW_OPEN_PREFIX, PLAN_HEADER, ROLE_SAY_HEADER, ROLE_USER_HEADER,
    STEP_ROW_CLOSED_PREFIX, STEP_ROW_OPEN_PREFIX, STEP_THINKING_HEADER,
};
use crate::markdown::{
    CODE_BOTTOM, CODE_ROW_EMPTY, CODE_ROW_PREFIX, CODE_TOP_PREFIX, QUOTE_PREFIX, RULE_LINE,
};

/// Shape of a rendered line, per the exact span structures the renderers
/// emit. Decoration kinds are dropped wholesale by [`clean_line`]; the two
/// content kinds keep their span payload minus the decoration slots.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum LineKind {
    /// Ordinary content row: keep, minus any leading slot.
    Text,
    /// `❯ User:` / `❯ Say:` role header — dropped.
    RoleHeader,
    /// `┌ label ` fenced-code top frame — dropped.
    CodeTop,
    /// `│ …` fenced-code content row — keep the payload verbatim.
    CodeRow,
    /// `└──…` fenced-code bottom frame — dropped.
    CodeBottom,
    /// `──…` thematic break — dropped.
    Rule,
    /// `╸─ plan ─╸` plan header — dropped.
    PlanHeader,
    /// `❯ Step(n)` / `▸ Step(n)` step row — dropped (chrome).
    StepRow,
    /// `❯ N steps` / `▸ N steps` group row (optionally + spinner span) —
    /// dropped (chrome).
    GroupRow,
    /// `❯ N function calls` / `▸ N function calls` calls aggregation row —
    /// dropped (chrome).
    CallsRow,
    /// `💭 Thinking` header row (standalone expanded block, or an open
    /// step's folded thinking) — dropped.
    ThinkingHeader,
}

/// Classify a rendered line and report its decoration slot width in
/// characters. Body rows inside indented blocks carry their gutter as a
/// separate all-space span, so the decoration shape (if any) is recognized
/// *after* it; the returned slot is the gutter width plus, for code rows,
/// the `│ `/`│` prefix width.
pub(crate) fn classify(line: &Line<'_>) -> (LineKind, usize) {
    match line.spans.first() {
        None => (LineKind::Text, 0),
        Some(first) => {
            let head = first.content.as_ref();
            match gutter_width(head) {
                Some(slot) => match classify_spans(&line.spans[1..]) {
                    Some((kind, extra)) => (kind, slot + extra),
                    None => (LineKind::Text, slot),
                },
                None => classify_spans(&line.spans).unwrap_or((LineKind::Text, 0)),
            }
        }
    }
}

/// Decoration shapes of a span slice (a whole line, or the remainder after a
/// gutter span): an exact single-span match for a frame/header/rule row, or a
/// leading code-row prefix span. Returns the kind plus the code-prefix slot.
fn classify_spans(spans: &[Span<'_>]) -> Option<(LineKind, usize)> {
    match spans.first().map(|s| s.content.as_ref()) {
        Some(CODE_ROW_PREFIX) => return Some((LineKind::CodeRow, CODE_ROW_PREFIX.chars().count())),
        Some(CODE_ROW_EMPTY) => return Some((LineKind::CodeRow, CODE_ROW_EMPTY.chars().count())),
        _ => {}
    }
    if let [only] = spans {
        let t = only.content.as_ref();
        if t == ROLE_USER_HEADER || t == ROLE_SAY_HEADER {
            return Some((LineKind::RoleHeader, 0));
        }
        if t == STEP_THINKING_HEADER {
            return Some((LineKind::ThinkingHeader, 0));
        }
        if t == RULE_LINE {
            return Some((LineKind::Rule, 0));
        }
        if t == CODE_BOTTOM {
            return Some((LineKind::CodeBottom, 0));
        }
        if t == PLAN_HEADER {
            return Some((LineKind::PlanHeader, 0));
        }
        // Step rows carry the step label as ONE span after the indent gutter;
        // a markdown row could only collide by literally opening with
        // `❯ Step(` / `▸ Step(` as its entire first span.
        if t.starts_with(STEP_ROW_OPEN_PREFIX) || t.starts_with(STEP_ROW_CLOSED_PREFIX) {
            return Some((LineKind::StepRow, 0));
        }
        // `┌ {label} ` with any (possibly empty) label. A code *content* row
        // can never match: it always carries a `│ ` prefix span first.
        if t.starts_with(CODE_TOP_PREFIX) && t.ends_with(' ') && !t.contains('\n') {
            return Some((LineKind::CodeTop, 0));
        }
    }
    if is_group_row(spans) {
        return Some((LineKind::GroupRow, 0));
    }
    if is_calls_row(spans) {
        return Some((LineKind::CallsRow, 0));
    }
    None
}

/// Match one fold-glyph count row — a single label span `{❯|▸} {count}{unit}`
/// (unit given as its plural/singular suffixes), optionally followed by the
/// `⠋ running ` spinner span — returning the count. Shared by the group row
/// (`N step(s)`) and the calls aggregation row (`N function call(s)`); both
/// labels are pure navigation chrome, so a markdown row could only collide
/// by being exactly such a label (same caveat as StepRow).
fn count_row_label(spans: &[Span<'_>], plural: &str, singular: &str) -> Option<u32> {
    let (label, spinner) = match spans {
        [label] => (label, None),
        [label, spinner] => (label, Some(spinner)),
        _ => return None,
    };
    if spinner.is_some_and(|sp| !sp.content.ends_with("running ")) {
        return None;
    }
    let t = label.content.as_ref();
    let body = t
        .strip_prefix(GROUP_ROW_OPEN_PREFIX)
        .or_else(|| t.strip_prefix(GROUP_ROW_CLOSED_PREFIX))?;
    let count = body
        .strip_suffix(plural)
        .or_else(|| body.strip_suffix(singular))?;
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    count.parse().ok()
}

/// `true` for the group's L0 row: one span `{❯|▸} N steps` (count non-empty,
/// digits), optionally followed by the `⠋ running ` spinner span.
fn is_group_row(spans: &[Span<'_>]) -> bool {
    count_row_label(spans, " steps", " step").is_some()
}

/// `true` for a step's calls aggregation row: one span
/// `{❯|▸} N function calls` (count non-empty, digits) — rendered behind the
/// step's 4-space gutter, which `classify` strips before matching here.
fn is_calls_row(spans: &[Span<'_>]) -> bool {
    count_row_label(spans, " function calls", " function call").is_some()
}

/// Width of `s` when it is a pure ASCII-space gutter span of 1..=8 columns.
fn gutter_width(s: &str) -> Option<usize> {
    let w = s.chars().count();
    if (1..=8).contains(&w) && s.bytes().all(|b| b == b' ') {
        Some(w)
    } else {
        None
    }
}

/// Concatenate a rendered `Line`'s span contents into plain text.
pub fn plain_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Clean one rendered row for the copy-mode view. Returns `None` for
/// pure-decoration rows (role/plan headers, code frames, thematic breaks);
/// otherwise the row's span payload with its decoration slots stripped
/// structurally and trailing padding trimmed. Code-row payloads are kept
/// verbatim apart from the `│ ` prefix; blank code rows yield `Some("")` so
/// interior blank lines of a fenced block survive.
pub fn clean_line(line: &Line<'_>) -> Option<String> {
    let (kind, _) = classify(line);
    match kind {
        LineKind::RoleHeader
        | LineKind::ThinkingHeader
        | LineKind::CodeTop
        | LineKind::CodeBottom
        | LineKind::Rule
        | LineKind::PlanHeader
        | LineKind::StepRow
        | LineKind::GroupRow
        | LineKind::CallsRow => None,
        // Code payloads stay verbatim (only the `│ `/`│` slot goes); text
        // rows additionally lose a `▎ ` quote prefix inside their first
        // content span, since blockquotes push it into the text span.
        LineKind::CodeRow => Some(payload_text(line, false)),
        LineKind::Text => Some(payload_text(line, true)),
    }
}

/// Concatenate the line's span payload after removing decoration slots
/// structurally: one optional pure-space gutter span, then (for code rows)
/// the `│ `/`│` prefix span, then — when `strip_quote_prefix` — a leading
/// `▎ ` inside the first content span. Trailing padding is trimmed.
fn payload_text(line: &Line<'_>, strip_quote_prefix: bool) -> String {
    let mut spans = skip_gutter(line.spans.as_slice());
    if !strip_quote_prefix {
        spans = skip_code_prefix(spans);
    }
    spans
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let content = s.content.as_ref();
            match i {
                0 if strip_quote_prefix => content.strip_prefix(QUOTE_PREFIX).unwrap_or(content),
                _ => content,
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Drop the leading span when it is a pure ASCII-space gutter span.
fn skip_gutter<'s, 't>(spans: &'s [Span<'t>]) -> &'s [Span<'t>] {
    match spans.first() {
        Some(s) if gutter_width(s.content.as_ref()).is_some() => &spans[1..],
        _ => spans,
    }
}

/// Drop the leading span when it is exactly the `│ `/`│` code-row prefix.
/// Compared by content only — the rendered prefix span is styled.
fn skip_code_prefix<'s, 't>(spans: &'s [Span<'t>]) -> &'s [Span<'t>] {
    match spans.first().map(|s| s.content.as_ref()) {
        Some(CODE_ROW_PREFIX) | Some(CODE_ROW_EMPTY) => &spans[1..],
        _ => spans,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A rendered line whose spans have exactly the contents `parts`.
    fn line_of(parts: &[&str]) -> Line<'static> {
        Line::from(
            parts
                .iter()
                .map(|p| Span::raw((*p).to_string()))
                .collect::<Vec<_>>(),
        )
    }

    /// Run a table of `(name, span contents, expected cleaned text)` cases.
    fn run(cases: &[(&'static str, &[&str], Option<&'static str>)]) {
        for (name, parts, want) in cases {
            let line = line_of(parts);
            assert_eq!(
                clean_line(&line).as_deref(),
                *want,
                "case {name:?}: spans = {:?}",
                line.spans
            );
        }
    }

    #[test]
    fn decoration_shapes_are_dropped() {
        run(&[
            ("role user header", &[ROLE_USER_HEADER], None),
            ("role say header", &[ROLE_SAY_HEADER], None),
            ("thinking header", &[STEP_THINKING_HEADER], None),
            ("plan header", &[PLAN_HEADER], None),
            ("rule line", &[RULE_LINE], None),
            ("code bottom frame", &[CODE_BOTTOM], None),
            ("code top with label", &["\u{250c} rust "], None),
            ("code top empty label", &["\u{250c}  "], None),
            // Indented bodies: the same shapes after a gutter span.
            (
                "role header behind 4-gutter",
                &["    ", ROLE_SAY_HEADER],
                None,
            ),
            // An open step's folded-thinking header sits behind a 4-gutter.
            (
                "thinking header behind 4-gutter",
                &["    ", STEP_THINKING_HEADER],
                None,
            ),
            ("rule behind 4-gutter", &["    ", RULE_LINE], None),
            (
                "code top behind 4-gutter",
                &["    ", "\u{250c} yaml "],
                None,
            ),
            ("code bottom behind 2-gutter", &["  ", CODE_BOTTOM], None),
            ("plan header behind 2-gutter", &["  ", PLAN_HEADER], None),
        ]);
    }

    #[test]
    fn decoration_glyphs_inside_code_rows_survive() {
        run(&[
            // `---` (YAML frontmatter) inside a fenced block is content.
            ("code row with ---", &[CODE_ROW_PREFIX, "---"], Some("---")),
            (
                "code row with rule glyphs",
                &[CODE_ROW_PREFIX, RULE_LINE],
                Some(RULE_LINE),
            ),
            (
                "code row with bottom frame",
                &[CODE_ROW_PREFIX, "\u{2514}\u{2500}\u{2500}"],
                Some("\u{2514}\u{2500}\u{2500}"),
            ),
            (
                "code row with top frame",
                &[CODE_ROW_PREFIX, "\u{250c} x "],
                Some("\u{250c} x"),
            ),
            // The same rows nested in an indented body (gutter + prefix).
            (
                "gutter code row with ---",
                &["    ", CODE_ROW_PREFIX, "---"],
                Some("---"),
            ),
            (
                "gutter code row with rule",
                &["    ", CODE_ROW_PREFIX, RULE_LINE],
                Some(RULE_LINE),
            ),
        ]);
    }

    #[test]
    fn slots_are_stripped_structurally() {
        run(&[
            // 4-space (user/assistant/image) and 2-space (plan) gutters are
            // separate spans — stripped exactly, no 4-vs-2 guessing.
            ("4-gutter", &["    ", "hello"], Some("hello")),
            ("2-gutter", &["  ", "plan line"], Some("plan line")),
            // Prose that itself starts with spaces keeps them: those spaces
            // live in the *content* span, beyond the exact gutter span.
            (
                "2-gutter keeps own lead",
                &["  ", "  nested"],
                Some("  nested"),
            ),
            (
                "4-gutter keeps own lead",
                &["    ", "    deep"],
                Some("    deep"),
            ),
            // Blockquote `▎ ` is pushed into the text span — strip it inside
            // the first content span, keep the text.
            (
                "quote behind gutter",
                &["    ", "\u{258e} quoted"],
                Some("quoted"),
            ),
            ("quote at slot 0", &["\u{258e} quoted"], Some("quoted")),
            // Gutter + code prefix compose (code inside an indented block).
            (
                "4-gutter + code prefix",
                &["    ", CODE_ROW_PREFIX, "fn main() {}"],
                Some("fn main() {}"),
            ),
        ]);
    }

    #[test]
    fn slotless_rows_pass_through_untouched() {
        run(&[
            (
                "tool header",
                &["\u{25b8} bash ls -la"],
                Some("\u{25b8} bash ls -la"),
            ),
            // A COLLAPSED standalone Thinking header carries the line count
            // in the same span, so it does not exact-match the chrome shape
            // and survives verbatim.
            (
                "collapsed thinking header keeps count",
                &["\u{1f4ad} Thinking (3 lines)"],
                Some("\u{1f4ad} Thinking (3 lines)"),
            ),
            // Tool/thinking body rows merge their 2-space lead into the
            // content span — no all-space gutter span, nothing to strip.
            (
                "tool body keeps its lead",
                &["  ls output"],
                Some("  ls output"),
            ),
            (
                "marker line",
                &["  \u{2714} subagent done"],
                Some("  \u{2714} subagent done"),
            ),
            // A bare `---`/`──` is NOT a separator here: the renderer's rule
            // row is exactly `─`×19; anything else is content.
            ("bare ---", &["---"], Some("---")),
            ("bare --verbose", &["--verbose"], Some("--verbose")),
            ("short dash run", &["────"], Some("────")),
            // Box-drawing glyphs leading prose stay (the old heuristic killed
            // any row merely *starting* with ┌/└).
            (
                "prose starting with └",
                &["└ not a frame"],
                Some("└ not a frame"),
            ),
            (
                "top glyph without frame shape",
                &["┌ not-a-frame"],
                Some("┌ not-a-frame"),
            ),
        ]);
    }

    #[test]
    fn empty_rows_and_padding() {
        run(&[
            // Empty fenced-code row: kept as an empty string so interior
            // blank lines of a code block survive.
            ("empty code row", &[CODE_ROW_EMPTY], Some("")),
            (
                "empty code row behind gutter",
                &["    ", CODE_ROW_EMPTY],
                Some(""),
            ),
            ("empty line", &[""], Some("")),
            ("pure gutter row", &["    "], Some("")),
            // Trailing padding from border filler is trimmed.
            ("trailing spaces", &["hi   "], Some("hi")),
        ]);
    }

    #[test]
    fn classify_reports_kinds_and_exact_slots() {
        let cases: &[(&[&str], LineKind, usize)] = &[
            (&[ROLE_USER_HEADER], LineKind::RoleHeader, 0),
            (&[RULE_LINE], LineKind::Rule, 0),
            (&[CODE_BOTTOM], LineKind::CodeBottom, 0),
            (&[PLAN_HEADER], LineKind::PlanHeader, 0),
            (&[STEP_THINKING_HEADER], LineKind::ThinkingHeader, 0),
            (&["    ", STEP_THINKING_HEADER], LineKind::ThinkingHeader, 4),
            (&["\u{25b8} 2 steps"], LineKind::GroupRow, 0),
            (&["\u{276f} 1 step"], LineKind::GroupRow, 0),
            (
                &["    ", "\u{25b8} 2 function calls"],
                LineKind::CallsRow,
                4,
            ),
            (&["\u{250c} rust "], LineKind::CodeTop, 0),
            (&[CODE_ROW_PREFIX, "x"], LineKind::CodeRow, 2),
            (&[CODE_ROW_EMPTY], LineKind::CodeRow, 1),
            (&["    ", CODE_ROW_PREFIX, "x"], LineKind::CodeRow, 6),
            (&["    ", "x"], LineKind::Text, 4),
            (&["  ", "x"], LineKind::Text, 2),
            (&["    ", RULE_LINE], LineKind::Rule, 4),
            (&["x y"], LineKind::Text, 0),
            (&[""], LineKind::Text, 0),
        ];
        for (parts, want_kind, want_slot) in cases {
            let line = line_of(parts);
            assert_eq!(
                classify(&line),
                (*want_kind, *want_slot),
                "spans = {:?}",
                line.spans
            );
        }
    }

    #[test]
    fn plain_text_concatenates_spans() {
        assert_eq!(plain_text(&line_of(&["a", "b", "c"])), "abc");
        assert_eq!(plain_text(&Line::from("")), "");
    }

    #[test]
    fn group_rows_are_dropped_with_and_without_spinner() {
        run(&[
            ("closed singular", &["\u{25b8} 1 step"], None),
            ("closed plural", &["\u{25b8} 3 steps"], None),
            ("open marker", &["\u{276f} 2 steps"], None),
            ("open singular", &["\u{276f} 1 step"], None),
            (
                "marker + spinner",
                &["\u{25b8} 2 steps", "\u{280b} running "],
                None,
            ),
            (
                "open marker + spinner",
                &["\u{276f} 1 step", "\u{2824} running "],
                None,
            ),
        ]);
    }

    #[test]
    fn calls_aggregation_rows_are_dropped() {
        run(&[
            (
                "closed singular",
                &["    ", "\u{25b8} 1 function call"],
                None,
            ),
            ("open plural", &["    ", "\u{276f} 3 function calls"], None),
            ("no gutter", &["\u{25b8} 2 function calls"], None),
            (
                "double-digit",
                &["    ", "\u{276f} 12 function calls"],
                None,
            ),
        ]);
    }

    #[test]
    fn group_row_lookalikes_survive() {
        run(&[
            ("no count", &["\u{25b8} step"], Some("\u{25b8} step")),
            (
                "non-digit count",
                &["\u{25b8} x steps"],
                Some("\u{25b8} x steps"),
            ),
            ("missing glyph", &["1 step"], Some("1 step")),
            // The old static marker glyph no longer opens a group row — a
            // leftover-looking span is plain content.
            (
                "old static marker is content",
                &["\u{2261} 2 steps"],
                Some("\u{2261} 2 steps"),
            ),
            (
                "spinner-less tail span is not a group row",
                &["\u{25b8} 2 steps", "extra"],
                Some("\u{25b8} 2 stepsextra"),
            ),
            // Three spans can never be a group row (label + spinner max).
            (
                "three spans",
                &["\u{25b8} 2 steps", "\u{280b} running ", "x"],
                Some("\u{25b8} 2 steps\u{280b} running x"),
            ),
            // Spinner span that does not end in `running ` is content.
            (
                "tail span is not a spinner",
                &["\u{25b8} 2 steps", "\u{280b} paused"],
                Some("\u{25b8} 2 steps\u{280b} paused"),
            ),
        ]);
    }
}
