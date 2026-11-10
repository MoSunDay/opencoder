//! Header line-accounting for Thinking / Subagent / step-row / Compaction
//! / turn-live-region blocks.
//!
//! Walks `ChatView.blocks` with the identical per-block line accounting that
//! `flatten_with` emits, so click hit-rects stay aligned with the live render.
//! Extracted from `chat.rs` for the line gate; behavior unchanged.

use super::live_turn;
use super::{
    step_render, ChatBlock, ChatView, CompactionHeader, LiveHeader, SubagentHeader, ThinkingHeader,
    ToolCallHeader,
};

impl ChatView {
    /// Return each Thinking block's `(block_idx, header_line_idx)`, where
    /// `header_line_idx` is the index in `flatten()` of its header line. Walks
    /// the blocks with the same per-block line accounting `flatten()` uses, so
    /// the indices stay in sync with what is rendered. Used by `render_body`
    /// to build click hit-rects.
    pub fn thinking_headers(&self) -> Vec<ThinkingHeader> {
        self.collect_headers().0
    }

    /// Return each Subagent block's `(block_idx, header_line_idx)`. Mirrors
    /// `thinking_headers`; used to build click hit-rects for subagent headers.
    pub fn subagent_headers(&self) -> Vec<SubagentHeader> {
        self.collect_headers().1
    }

    /// Return each clickable row inside `StepGroup` blocks — the turn row,
    /// the step rows, calls aggregation rows, and function-call header rows,
    /// in render order — as `(block_idx, call_idx, header_line_idx)`.
    /// `call_idx` is the FLAT index into that visible-row walk (shared shape
    /// with `super::steps::visible_targets`), so clicking resolves exactly
    /// the rendered row.
    pub fn tool_call_headers(&self) -> Vec<ToolCallHeader> {
        self.collect_headers().3
    }

    /// Return the TURN LIVE REGION's header rows — one per RUNNING group
    /// (`progress_active`, steps in flight) — in render order. Clicking one
    /// toggles the region between its detail rows and the collapsed
    /// `▸ streaming` header (`toggle_live_region`).
    pub fn live_headers(&self) -> Vec<LiveHeader> {
        self.collect_headers().4
    }

    /// Single pass over all blocks computing the header line index of every
    /// Thinking / Subagent / step-row block, using the identical per-block
    /// line accounting that `flatten_with()` emits. Keeping the accounting in
    /// one place guarantees hit-rect indices stay aligned with the live
    /// render.
    #[allow(clippy::type_complexity)]
    pub(crate) fn collect_headers(
        &self,
    ) -> (
        Vec<ThinkingHeader>,
        Vec<SubagentHeader>,
        Vec<CompactionHeader>,
        Vec<ToolCallHeader>,
        Vec<LiveHeader>,
    ) {
        let mut think = Vec::new();
        let mut sub = Vec::new();
        let mut compaction = Vec::new();
        let mut tool_calls = Vec::new();
        let mut live = Vec::new();
        let mut line_idx = 0usize;
        let merged_say = |i: usize| -> bool {
            // Only a SEALED follower merges (`done == true`) — mirrors
            // `flatten_with`'s StepGroup adjacency lookup. An OPEN follower
            // retires the whole ladder instead (`say_streams` in the arm).
            matches!(
                self.blocks.get(i + 1),
                Some(ChatBlock::Assistant { done: true, .. })
            )
        };
        for (block_idx, block) in self.blocks.iter().enumerate() {
            match block {
                ChatBlock::Marker(lines) => line_idx += lines.len(),
                ChatBlock::User { rendered } => line_idx += 1 + rendered.len(),
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
                    // The Say is transcript content BOTH while streaming
                    // and once sealed — mirrors `flatten_with`'s Assistant
                    // arm line for line. While it streams (`done == false`)
                    // the Say renders STANDALONE: +1 for the `say:` header
                    // row plus every raw body row (no merge, no preview
                    // dedup). Once sealed, `merged` looks at the PRECEDING
                    // block (a StepGroup), mirroring flatten's adjacency.
                    if !*done {
                        line_idx += 1; // standalone `say:` header row
                        line_idx += super::assistant_rows(raw).len();
                        continue;
                    }
                    let merged = matches!(
                        block_idx.checked_sub(1).and_then(|i| self.blocks.get(i)),
                        Some(ChatBlock::StepGroup { .. })
                    );
                    // +1 for the "say:" header line emitted by flatten() —
                    // skipped when this Say is merged into the preceding
                    // StepGroup's header row (the group emitted the merged
                    // `Say(n steps)` row and this block renders body only).
                    if !merged {
                        line_idx += 1;
                    }
                    // 合并对正文行数镜像：跳过与 preview 重复的首个非空行
                    // （单行 Say / 空正文整块隐藏）—— 与 `flatten_with` 的
                    // Assistant 分支逐行同步，hit-rect 才能对齐。
                    line_idx += if merged {
                        step_render::merged_say_body_decision(rendered).visible_len(rendered.len())
                    } else {
                        rendered.len()
                    };
                }
                ChatBlock::Thinking {
                    text, collapsed, ..
                } => {
                    think.push(ThinkingHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    // Header line always emitted; content lines only when expanded.
                    line_idx += 1;
                    if !collapsed {
                        line_idx += text.lines().count();
                    }
                }
                ChatBlock::StepGroup {
                    steps,
                    open,
                    progress_active,
                    ..
                } => {
                    // Mirrors `flatten_step_group` exactly (three-level
                    // ladder): the turn row always (a clickable target);
                    // while the turn is open, per step the step row (target),
                    // and while THAT step is open its `💭 Thinking` header +
                    // thinking lines plus the calls aggregation row (target),
                    // then each call header while that list is open (target;
                    // + result + blank when the call is expanded); one trailing
                    // blank, merged into the final expanded call's separator.
                    //
                    // ADJACENT-pair merge: when the next block is this turn's
                    // Say (`Assistant`), the standalone `N Steps` row is
                    // replaced by the merged `Say(n steps): <preview>` header
                    // — the group target (call_idx 0) stays on that row, and
                    // the header's separator blank right below terminates the
                    // CLOSED pair (no ladder rows, no second trailing blank;
                    // the Say body follows with its first line deduped
                    // against the preview).
                    //
                    // Ladder gating mirror (`steps::settled_steps`): while
                    // the group runs, the in-flight LAST step streams only
                    // in the TURN LIVE REGION below the ladder — no ladder
                    // rows and no targets for it here; the region's rows are
                    // counted (and its header registered as a target) so
                    // later blocks' header rows stay aligned with flatten.
                    let say_merged = merged_say(block_idx);
                    let say_streams = matches!(
                        self.blocks.get(block_idx + 1),
                        Some(ChatBlock::Assistant { done: false, .. })
                    );
                    let ladder = super::steps::settled_steps(*progress_active, steps);
                    let live_rows = if *progress_active && !steps.is_empty() {
                        // `anim_tick` never changes the region's line count.
                        live_turn::render_lines(steps, 0, self.live_collapsed).len()
                    } else {
                        0
                    };
                    if (*progress_active && ladder.is_empty()) || say_streams {
                        // First step still streaming OR the turn's Say
                        // streaming below: the ladder RETIRED (flatten
                        // skips `flatten_step_group`) — mirror that here:
                        // no group row, no blank, no targets; only the
                        // live region's rows (normally already disarmed)
                        // still count.
                    } else {
                        let mut call_idx = 0usize;
                        tool_calls.push(ToolCallHeader {
                            block_idx,
                            call_idx,
                            header_line_idx: line_idx,
                        });
                        call_idx += 1;
                        line_idx += 1; // group row (or merged Say header row)
                        if say_merged {
                            // 合并头部行之后的空行（与 `flatten_step_group` 同
                            // 步）：闭合时兼任整对的尾部空行，展开时隔开头部
                            // 与 ladder。
                            line_idx += 1;
                        }
                        if *open {
                            for step in ladder.iter() {
                                tool_calls.push(ToolCallHeader {
                                    block_idx,
                                    call_idx,
                                    header_line_idx: line_idx,
                                });
                                call_idx += 1;
                                line_idx += 1; // step row
                                if step.open {
                                    if !step.thinking.is_empty() {
                                        line_idx += 1 + step.thinking.len();
                                    }
                                    if !step.calls.is_empty() {
                                        tool_calls.push(ToolCallHeader {
                                            block_idx,
                                            call_idx,
                                            header_line_idx: line_idx,
                                        });
                                        call_idx += 1;
                                        line_idx += 1; // calls aggregation row
                                        if step.calls_open {
                                            for c in &step.calls {
                                                tool_calls.push(ToolCallHeader {
                                                    block_idx,
                                                    call_idx,
                                                    header_line_idx: line_idx,
                                                });
                                                call_idx += 1;
                                                line_idx += 1 + if c.expanded {
                                                    1 + c.output.len()
                                                } else {
                                                    0
                                                };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Exactly one blank after the group: when the last
                        // visible row is an expanded call, its separator blank
                        // (counted above) IS the trailing blank — don't add
                        // another.
                        let ends_on_expanded_call = *open
                            && ladder.last().is_some_and(|s| {
                                s.open && s.calls_open && s.calls.last().is_some_and(|c| c.expanded)
                            });
                        if !ends_on_expanded_call && (!say_merged || *open) {
                            line_idx += 1; // trailing blank
                        }
                    }
                    // The live region's rows ride right below the ladder's
                    // trailing blank (or alone while the first step still
                    // streams) — mirror of the flatten emission. Its header
                    // row is the region's click target.
                    if live_rows > 0 {
                        live.push(LiveHeader {
                            block_idx,
                            header_line_idx: line_idx,
                        });
                    }
                    line_idx += live_rows;
                }
                ChatBlock::Compaction {
                    text, collapsed, ..
                } => {
                    compaction.push(CompactionHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    line_idx += 1;
                    if !collapsed {
                        line_idx += text.lines().count();
                    }
                }
                ChatBlock::Image { rendered, .. } => {
                    // Empty `rendered` -> flatten_with emits a placeholder line; count 1 not 0.
                    line_idx +=
                        1 + if rendered.is_empty() {
                            1
                        } else {
                            rendered.len()
                        } + 1;
                }
                ChatBlock::Subagent { .. } => {
                    sub.push(SubagentHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    line_idx += 1; // header only — no inline expansion
                }
                ChatBlock::Plan { rendered, .. } => {
                    line_idx += 1 + rendered.len() + 1;
                }
            }
        }
        // TURN LIVE REGION mirror-note: while the admitted turn runs, its
        // CURRENT (in-flight) step streams in the region right below the
        // running ladder (`chat_live_turn.rs`) — counted above (and its
        // header registered as a click target) so hit-rects stay aligned
        // with the flatten output. It retires when progress settles; the
        // open Say, by contrast, IS transcript content (accounted above —
        // raw rows while it streams, rendered rows once sealed).
        (think, sub, compaction, tool_calls, live)
    }
}
