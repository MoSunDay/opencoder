//! Transcript flatten — the rendering half of `ChatView`, split from
//! `chat.rs` for the line gate (second `impl ChatView`, same pattern as
//! `chat_plan.rs`). `flatten_with` is the SINGLE source of rendered line
//! shapes: `collect_headers` mirrors its per-block accounting so mouse
//! hit-rects stay aligned with the live render.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{
    assistant_rows, push_duration_span, render_collapsible, step_render, SayHeader, PLAN_HEADER,
    ROLE_SAY_HEADER, ROLE_USER_HEADER, SPINNER,
};
use super::{theme, types, ChatBlock, ChatView};

impl ChatView {
    /// Flatten all blocks into a single `Vec<Line>` for rendering, using
    /// `anim_tick` advances running subagent and step-progress spinners. Delegated to
    /// by `flatten()` (which passes `0`) for non-render callers (selection,
    /// scroll-counting, tests) — line counts are identical across tick values,
    /// so hit-rects and selection math stay aligned with the live render.
    pub fn flatten_with(&self, anim_tick: u32, now_ms: i64) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.blocks.len() * 2);
        for (bi, block) in self.blocks.iter().enumerate() {
            match block {
                ChatBlock::Marker(lines) => out.extend(lines.iter().cloned()),
                ChatBlock::User { rendered } => {
                    out.push(Line::from(Span::styled(
                        ROLE_USER_HEADER,
                        Style::default()
                            .fg(theme::user_color())
                            .add_modifier(Modifier::BOLD),
                    )));
                    out.extend(types::indented(rendered, 4));
                }
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
                    // The Say is TRANSCRIPT CONTENT both while streaming
                    // and once sealed: an open Say (`done == false`)
                    // streams its RAW body rows right here (same shape as
                    // sealed), sharing the transcript's scroll. It renders
                    // in its FINAL shape immediately — under a preceding
                    // StepGroup the merged `❯ Say(n steps)` header row
                    // (spinner running), else the standalone `say:`
                    // header — so sealing (raw rows swap for the rendered
                    // ones) causes zero visual jump.
                    let merged = matches!(
                        bi.checked_sub(1).and_then(|i| self.blocks.get(i)),
                        Some(ChatBlock::StepGroup { .. })
                    );
                    if !merged {
                        out.push(Line::from(Span::styled(
                            ROLE_SAY_HEADER,
                            Style::default()
                                .fg(theme::ok_color())
                                .add_modifier(Modifier::BOLD),
                        )));
                    }
                    // 合并对正文去重：头部行的 preview 已展示正文首个非空
                    // 行，正文不再重复输出该行；单行 Say 则整块不渲染。
                    // 流式时 preview/正文都取 raw（`done == false` 路径）。
                    let decision = if merged {
                        step_render::merged_say_body_decision(raw, rendered, *done)
                    } else {
                        step_render::SayBody::Full
                    };
                    let rows: Vec<Line<'static>> = if *done {
                        rendered.clone()
                    } else {
                        assistant_rows(raw)
                            .into_iter()
                            .map(|l| Line::from(l.to_string()))
                            .collect()
                    };
                    let visible: &[Line<'static>] = match decision {
                        step_render::SayBody::Hidden => &[],
                        step_render::SayBody::Skip(n) => &rows[n.min(rows.len())..],
                        step_render::SayBody::Full => rows.as_slice(),
                    };
                    out.extend(types::indented(visible, 4));
                }
                ChatBlock::Thinking {
                    text, collapsed, ..
                } => {
                    out.extend(render_collapsible(
                        "\u{1f4ad}",
                        "Thinking",
                        text,
                        *collapsed,
                        Style::default()
                            .fg(theme::pink())
                            .add_modifier(Modifier::BOLD),
                        Style::default().fg(theme::muted()),
                    ));
                }
                ChatBlock::Compaction {
                    text, collapsed, ..
                } => {
                    out.extend(render_collapsible(
                        "\u{1f4dd}",
                        "Compaction",
                        text,
                        *collapsed,
                        Style::default().fg(theme::compaction_color()),
                        Style::default().fg(theme::compaction_color()),
                    ));
                }
                ChatBlock::StepGroup {
                    steps,
                    open,
                    progress_active,
                    ..
                } => {
                    // Adjacent-pair merge: when the block right below this
                    // group is the turn's Say (ANY `Assistant`, open or
                    // sealed), the two render as one clickable header row —
                    // pass the Say so `flatten_step_group` can fold the
                    // `N Steps` row into it. An OPEN follower is transcript
                    // content now too: the merged `❯ Say(n steps)` header
                    // forms IMMEDIATELY with the running spinner, its raw
                    // body rows stream below (emitted by the Assistant
                    // arm), and it seals in place with zero visual jump.
                    let say = match self.blocks.get(bi + 1) {
                        Some(ChatBlock::Assistant {
                            raw,
                            rendered,
                            done,
                            ..
                        }) => Some(SayHeader {
                            preview: step_render::say_preview_for(raw, rendered, *done),
                            // `running` rides the merged header exactly
                            // while the Say streams (`done == false`);
                            // sealing drops the hint, nothing else moves.
                            streaming: !*done,
                        }),
                        _ => None,
                    };
                    step_render::flatten_step_group(
                        &mut out,
                        *open,
                        *progress_active,
                        steps,
                        anim_tick,
                        say,
                    );
                }
                ChatBlock::Image { filename, rendered } => {
                    out.push(Line::from(Span::styled(
                        format!("[image: {filename}]"),
                        Style::default().fg(theme::muted()),
                    )));
                    if rendered.is_empty() {
                        out.push(Line::from(Span::styled(
                            "  (unable to render)",
                            Style::default().fg(theme::muted()),
                        )));
                    } else {
                        out.extend(types::indented(rendered, 4));
                    }
                    out.push(Line::from(""));
                }
                ChatBlock::Plan { rendered, .. } => {
                    out.push(Line::from(Span::styled(
                        PLAN_HEADER,
                        Style::default()
                            .fg(theme::warn_color())
                            .add_modifier(Modifier::BOLD),
                    )));
                    out.extend(types::indented(rendered, 2));
                    out.push(Line::from(""));
                }
                ChatBlock::Subagent {
                    kind,
                    prompt,
                    view,
                    done,
                    ok,
                    cancelled,
                    summary,
                    started_at_ms,
                    elapsed_ms,
                    ..
                } => {
                    let step_count = view
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            ChatBlock::StepGroup { steps, .. } => Some(steps.len()),
                            _ => None,
                        })
                        .sum::<usize>();
                    // Status badge: animated spinner/check/cross/cancelled +
                    // word. The running spinner uses the live anim_tick.
                    let (mark, mark_color, status_word) = if *cancelled {
                        ("\u{2298}", theme::muted(), "cancelled")
                    } else if *done {
                        if *ok {
                            ("\u{2714}", theme::ok_color(), "done")
                        } else {
                            ("\u{2718}", theme::err_color(), "failed")
                        }
                    } else {
                        (
                            SPINNER[(anim_tick as usize) % SPINNER.len()],
                            theme::warn_color(),
                            "running",
                        )
                    };
                    let mut spans = vec![
                        Span::styled(
                            "\u{2937} subagent ",
                            Style::default()
                                .fg(theme::info_color())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("[{kind}] "), Style::default().fg(theme::accent())),
                        Span::styled(prompt.clone(), Style::default().fg(theme::muted())),
                        Span::raw(" "),
                        Span::styled(
                            format!("{mark} {status_word}, {step_count} Steps"),
                            Style::default().fg(mark_color),
                        ),
                    ];
                    push_duration_span(&mut spans, *started_at_ms, *elapsed_ms, now_ms);
                    spans.push(Span::styled(
                        " [\u{2192} view]",
                        Style::default().fg(theme::muted()),
                    ));
                    if *done && !summary.is_empty() {
                        spans.push(Span::styled(
                            format!("  {summary}"),
                            Style::default().fg(if *cancelled || *ok {
                                theme::muted()
                            } else {
                                theme::err_color()
                            }),
                        ));
                    }
                    out.push(Line::from(spans));
                }
            }
        }
        // The transcript holds ONLY Say + ladder blocks — the running
        // turn's current step streams in the separate TURN LIVE REGION
        // (own layout area, see `chat_live_turn.rs` / `render.rs`), NEVER
        // in these lines: scroll, selection, hit-rects and line accounting
        // stay untouched by streaming.
        out
    }

    /// Non-animated flatten for callers that don't render (selection extract,
    /// scroll-counting, tests). Line counts match `flatten_with` exactly.
    pub fn flatten(&self) -> Vec<Line<'static>> {
        self.flatten_with(0, opencoder_core::message::now_ms())
    }

    /// True when the transcript's LAST rendered row is already blank —
    /// blank markers, or blocks that self-terminate with exactly one blank
    /// (StepGroup / Image / Plan; a Say only when its own body ends blank).
    /// Turn-boundary markers consult this so a boundary never stacks a second
    /// blank onto an output that already ends with one.
    pub(super) fn last_block_ends_blank(&self) -> bool {
        fn blank_line(l: &Line<'static>) -> bool {
            l.spans.iter().all(|s| s.content.trim().is_empty())
        }
        // 合并对（上一个块是 StepGroup）且正文整块隐藏（单行 Say / 空正文）
        // 时，整对以头部后的空行（闭合）或 ladder 尾部空行（展开）收尾，
        // 已经是 blank —— 边界不得再叠加第二个空行。
        if let Some(ChatBlock::Assistant {
            raw,
            rendered,
            done,
        }) = self.blocks.last()
        {
            let merged = matches!(
                self.blocks
                    .len()
                    .checked_sub(2)
                    .and_then(|i| self.blocks.get(i)),
                Some(ChatBlock::StepGroup { .. })
            );
            if merged
                && step_render::merged_say_body_decision(raw, rendered, *done)
                    == step_render::SayBody::Hidden
            {
                return true;
            }
        }
        match self.blocks.last() {
            None => true,
            Some(ChatBlock::Marker(lines)) => lines.last().is_some_and(blank_line),
            // These blocks always end with their own trailing blank.
            Some(ChatBlock::StepGroup { .. })
            | Some(ChatBlock::Image { .. })
            | Some(ChatBlock::Plan { .. }) => true,
            Some(ChatBlock::User { rendered }) => rendered.last().is_some_and(blank_line),
            Some(ChatBlock::Assistant {
                raw,
                rendered,
                done,
            }) => {
                if *done {
                    rendered.last().is_some_and(blank_line)
                } else {
                    assistant_rows(raw)
                        .last()
                        .is_some_and(|l| l.trim().is_empty())
                }
            }
            _ => false,
        }
    }
}
