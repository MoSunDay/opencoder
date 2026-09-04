//! Flattening for `ChatBlock::StepGroup` — the three-level tool ladder
//! (turn row → step content/calls aggregate → function-call result).
//! Extracted from `chat.rs` for the line gate; `collect_headers`
//! (chat_headers.rs) mirrors this line accounting exactly so hit-rects stay
//! aligned with the live render.
//!
//! Say pairing: a ladder whose ADJACENT lower block is its `Assistant` Say
//! renders as ONE header row `{glyph} Say(n steps): <say first line>` that
//! both carries the step count and toggles the steps on click; the
//! standalone `{glyph} N Steps` row disappears into it. The live `running`
//! hint survives the Say: it rides the merged header while the Say itself
//! streams and nothing was appended after it, then moves on (Done, or the
//! next ladder's own group row).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{
    theme, Step, ToolCall, GROUP_ROW_CLOSED_PREFIX, GROUP_ROW_OPEN_PREFIX, SPINNER,
    STEP_ROW_CLOSED_PREFIX, STEP_ROW_OPEN_PREFIX, STEP_THINKING_HEADER,
};

/// Append one `StepGroup` block's lines to `out`. Shape (three-level
/// drill-down): the clickable turn row `{▸|❯} N Steps` (col 0, accent
/// bold) + a live progress hint until Say begins; while the group is closed
/// that row (plus one trailing
/// blank) is the whole block. While it is open, per step: the step row
/// (indent 2); while the step is open its `💭 Thinking` block (header
/// indent 4, body indent 8) and a `N Function calls` aggregation row
/// (indent 4); opening the aggregation shows call headers at indent 6, and
/// an expanded call shows its result at indent 6. Exactly one trailing blank
/// line: the group's trailing blank is the ONLY separator after the final
/// expanded call's result — the per-call separator merges into it instead of
/// doubling (`User:`-block parity).
///
/// With `say` present (the ADJACENT lower `Assistant` block, see
/// [`SayHeader`]) the whole pair renders as one header row followed by ONE
/// separator blank, and — while the ladder is open — the step rows between
/// them; the Say body below (emitted by the Assistant block) skips its first
/// non-empty line when it duplicates the header preview (see
/// [`merged_say_body_decision`]).
/// Render-time view of the group's ADJACENT Say (the `Assistant` block
/// immediately below the group). When present, the pair renders as ONE
/// clickable header row `{glyph} Say(n steps): <say first line>`: the step
/// count folds into the Say header, the standalone `N Steps` row disappears,
/// and clicking the row toggles the ladder (the Say body always stays
/// visible below). `streaming` keeps the `running` hint alive on this row
/// while the Say itself streams with nothing appended after it — the hint
/// leaves the pre-Say group row only to move here, not to vanish.
pub(crate) struct SayHeader {
    /// Header inline preview, computed once via [`say_preview_for`]: the
    /// first non-empty line RENDERED (markdown applied) once the Say is
    /// done, the raw first line while it streams. The body dedup compares
    /// against the SAME string, so the header never shows raw markdown and
    /// never repeats itself below.
    pub preview: String,
    pub streaming: bool,
}

/// First non-empty line of the Say's raw text, trimmed — the inline preview
/// on the merged header (empty for a whitespace-only Say).
pub(crate) fn say_preview(raw: &str) -> &str {
    raw.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// done Say 的渲染结果里首个非空行文本（trim）：markdown 渲染后的口径
/// 与 [`merged_say_body`] 逐行取的 `line_text` 完全一致。
fn rendered_preview(rendered: &[Line<'static>]) -> String {
    rendered
        .iter()
        .map(line_text)
        .map(|t| t.trim().to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
}

/// 合并对头部 preview 的唯一口径：done 取 markdown 渲染后的首个非空行
/// （头部不露 `#`/`**`/`-` 这类原始标记，正文也不再把该行换个形态重复
/// 一遍）；流式取 raw 首行（与流式正文行同源，同口径去重）。done 但
/// 渲染结果为空时回退 raw，保证头部始终有 preview。
pub(crate) fn say_preview_for(raw: &str, rendered: &[Line<'static>], done: bool) -> String {
    if done {
        let rendered = rendered_preview(rendered);
        if rendered.is_empty() {
            say_preview(raw).to_string()
        } else {
            rendered
        }
    } else {
        say_preview(raw).to_string()
    }
}

/// 合并对里 Say 正文的去重判定（三态）。头部行已经用 preview 展示了正文
/// 的首个非空行，正文再原样输出该行就是一字不差的重复：
/// - [`SayBody::Full`]    首个非空行与 preview 不相等（trim 相等口径；
///   preview 若被截断也只按完整相等跳过，不做前缀匹配）→ 正文完整渲染；
/// - [`SayBody::Skip(n)`] 首个非空行就是 preview → 跳过它（连同其前导
///   空行共 n 行），其余照常渲染；
/// - [`SayBody::Hidden`]  跳过之后正文为空（单行 Say）或只剩空行 → 整块
///   不渲染，头部后的空行即充当整对的分隔与尾部空行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SayBody {
    Full,
    Skip(usize),
    Hidden,
}

impl SayBody {
    /// 正文可见行数 —— 计数镜像（`collect_headers`）与渲染（`flatten_with`）
    /// 共用同一口径，hit-rect 才能逐行对齐。
    pub(crate) fn visible_len(self, total: usize) -> usize {
        match self {
            SayBody::Full => total,
            SayBody::Skip(n) => total - n,
            SayBody::Hidden => 0,
        }
    }
}

/// 逐行扫描正文，返回去重三态（见 [`SayBody`]）。`rows` 为正文行文本。
pub(crate) fn merged_say_body(preview: &str, rows: &[impl AsRef<str>]) -> SayBody {
    let preview = preview.trim();
    for (i, row) in rows.iter().enumerate() {
        let text = row.as_ref().trim();
        if text.is_empty() {
            // 跳过前导空行，定位正文首个非空行。
            continue;
        }
        if text != preview {
            return SayBody::Full;
        }
        // 首个非空行即 preview：去重后其余行全空（含单行 Say 的空余量）
        // 则整块隐藏，避免在头部空行下方再叠一串空行。
        let rest_blank = rows[i + 1..].iter().all(|r| r.as_ref().trim().is_empty());
        return if rest_blank {
            SayBody::Hidden
        } else {
            SayBody::Skip(i + 1)
        };
    }
    // 正文全空/全空行：没有可见内容，按隐藏处理（保持「恰好一个尾部
    // 空行」不变量 —— 头部空行即收尾，边界标记不得再叠加第二个）。
    SayBody::Hidden
}

/// 单行 `Line` 的纯文本（span 内容拼接），供 done 正文的 trim 比较。
fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.clone()).collect()
}

/// 合并对正文的去重判定入口：done 时逐行取 markdown 渲染文本，流式时
/// 取原始行（与 `flatten_with` 两条正文路径一一对应），再交给
/// [`merged_say_body`]。渲染与行数统计都从这里走，保证两端口径一致。
pub(crate) fn merged_say_body_decision(
    raw: &str,
    rendered: &[Line<'static>],
    done: bool,
) -> SayBody {
    let preview = say_preview_for(raw, rendered, done);
    if done {
        let texts: Vec<String> = rendered.iter().map(line_text).collect();
        merged_say_body(&preview, &texts)
    } else {
        merged_say_body(&preview, &super::assistant_rows(raw))
    }
}

/// The merged pair header: `{glyph} Say(n step{s}): <preview>` plus the live
/// spinner while the Say streams. Same glyph grammar as the group row
/// (`❯` open / `▸` closed) so every collapsible row reads alike.
fn say_header_line(open: bool, n: usize, say: &SayHeader, anim_tick: u32) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!(
            "{}Say({} step{}): ",
            if open {
                GROUP_ROW_OPEN_PREFIX
            } else {
                GROUP_ROW_CLOSED_PREFIX
            },
            n,
            if n == 1 { "" } else { "s" }
        ),
        Style::default()
            .fg(theme::ok_color())
            .add_modifier(Modifier::BOLD),
    )];
    if !say.preview.is_empty() {
        spans.push(Span::raw(say.preview.clone()));
    }
    if say.streaming {
        spans.push(Span::styled(
            format!(
                "  {} running ",
                SPINNER[(anim_tick as usize) % SPINNER.len()]
            ),
            Style::default().fg(theme::warn_color()),
        ));
    }
    Line::from(spans)
}

pub(crate) fn flatten_step_group(
    out: &mut Vec<Line<'static>>,
    open: bool,
    progress_active: bool,
    steps: &[Step],
    anim_tick: u32,
    say: Option<SayHeader>,
) {
    let n = steps.len();
    match say {
        // Say-paired ladder: ONE header row `{glyph} Say(n step{s}): <say
        // first line>` carries the step count and toggles the ladder; the
        // standalone `N Steps` row disappears into it. While closed that row
        // plus the Say body (emitted by the Assistant block right below) is
        // the whole pair — no extra blank row.
        Some(say) => {
            out.push(say_header_line(open, steps.len(), &say, anim_tick));
            // 头部行之后必须空出一行再接下方内容：preview 行不与正文
            // （或展开的 ladder）挤在一起。闭合时这一个空行兼任整对的
            // 尾部空行 —— 正文若整块隐藏（单行 Say），绝不叠加第二个
            // 空行；展开时它隔开头部与 ladder，ladder 自身的尾部空行
            // 仍恰好一个（见下方）。
            out.push(Line::from(""));
            if !open {
                return;
            }
        }
        None => {
            // L0 group row: `{▸|❯} N Steps` + a live spinner hint from
            // step/tool activity until the next Say starts. The two leading
            // spaces keep motion visually separate from the count without
            // adding a row.
            let mut spans = vec![Span::styled(
                format!(
                    "{}{n} Step{}",
                    if open {
                        GROUP_ROW_OPEN_PREFIX
                    } else {
                        GROUP_ROW_CLOSED_PREFIX
                    },
                    if n == 1 { "" } else { "s" }
                ),
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD),
            )];
            if progress_active {
                spans.push(Span::styled(
                    format!(
                        "  {} running ",
                        SPINNER[(anim_tick as usize) % SPINNER.len()]
                    ),
                    Style::default().fg(theme::warn_color()),
                ));
            }
            out.push(Line::from(spans));
            if !open {
                out.push(Line::from(""));
                return;
            }
        }
    }
    for (si, step) in steps.iter().enumerate() {
        let step_open = step.open;
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(
                    "{}{})",
                    if step_open {
                        STEP_ROW_OPEN_PREFIX
                    } else {
                        STEP_ROW_CLOSED_PREFIX
                    },
                    si + 1,
                ),
                Style::default()
                    .fg(theme::ok_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if !step_open {
            continue;
        }
        if !step.thinking.is_empty() {
            out.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    STEP_THINKING_HEADER,
                    Style::default()
                        .fg(theme::pink())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            out.extend(super::types::indented(&step.thinking, 8));
        }
        if step.calls.is_empty() {
            continue;
        }
        let m = step.calls.len();
        out.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!(
                    "{}{m} Function call{}",
                    if step.calls_open {
                        GROUP_ROW_OPEN_PREFIX
                    } else {
                        GROUP_ROW_CLOSED_PREFIX
                    },
                    if m == 1 { "" } else { "s" }
                ),
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if !step.calls_open {
            continue;
        }
        for (ci, c) in step.calls.iter().enumerate() {
            let header = call_header(c);
            out.extend(super::types::indented(std::slice::from_ref(&header), 6));
            // Per-call expansion: only the toggled call shows its output.
            // Exactly one blank follows the result: for the group's final
            // visible row the group's trailing blank below already provides
            // it, so the per-call separator is skipped instead of doubling.
            if c.expanded {
                out.extend(super::types::indented(&c.output, 6));
                let group_final = si + 1 == steps.len() && ci + 1 == step.calls.len();
                if !group_final {
                    out.push(Line::from(""));
                }
            }
        }
    }
    out.push(Line::from(""));
}

/// Derive the disclosure glyph without mutating the stored call header.
fn call_header(call: &ToolCall) -> Line<'static> {
    let mut header = call.header.clone();
    let Some(first) = header.spans.first_mut() else {
        return header;
    };
    let text = first.content.to_string();
    let prefix = if call.expanded {
        GROUP_ROW_OPEN_PREFIX
    } else {
        GROUP_ROW_CLOSED_PREFIX
    };
    if let Some(body) = text
        .strip_prefix(GROUP_ROW_OPEN_PREFIX)
        .or_else(|| text.strip_prefix(GROUP_ROW_CLOSED_PREFIX))
    {
        first.content = format!("{prefix}{body}").into();
    }
    header
}
