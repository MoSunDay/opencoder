use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme;

use opencoder_llm::estimate;
use opencoder_session::SessionEvent;

#[path = "chat_types.rs"]
mod types;
pub use types::*;

#[path = "chat_helpers.rs"]
mod helpers;
pub use helpers::block_text;
pub(crate) use helpers::{short, summarize};

#[path = "compaction_block.rs"]
mod compaction_block;
pub(crate) use compaction_block::render_collapsible;

impl ChatView {
    pub fn apply(&mut self, ev: &SessionEvent) {
        self.track_context(ev);
        match ev {
            SessionEvent::TextDelta(t) => {
                self.ensure_assistant_open();
                if let Some(ChatBlock::Assistant { raw, .. }) = self.blocks.last_mut() {
                    raw.push_str(t);
                }
            }
            SessionEvent::ReasoningDelta(t) => {
                self.ensure_thinking_open();
                if let Some(ChatBlock::Thinking { text, .. }) = self.blocks.last_mut() {
                    text.push_str(t);
                }
            }
            // Stream the summary into an expanded block so it is visible while
            // the summarizing LLM call runs. The final `Compaction(summary)`
            // event (below) finalizes + collapses it.
            SessionEvent::CompactionDelta(t) => {
                self.open_compaction_streaming(t);
            }
            SessionEvent::ToolStart { id, name, input } => {
                if name == "task" {
                    return;
                }
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Tool {
                    id: id.clone(),
                    header: Line::from(vec![
                        Span::styled(
                            format!("\u{25b8} {name} "),
                            Style::default()
                                .fg(theme::accent())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(summarize(input), Style::default().fg(theme::muted())),
                    ]),
                    output: Vec::new(),
                    collapsed: true,
                });
            }
            SessionEvent::ToolEnd {
                id,
                name,
                output,
                is_error,
                images,
            } => {
                if name == "task" {
                    return;
                }
                self.finalize_assistant();
                let color = if *is_error {
                    theme::err_color()
                } else {
                    theme::muted()
                };
                let out: Vec<Line<'static>> = output
                    .lines()
                    .take(TOOL_OUTPUT_LINES)
                    .map(|l| Line::from(Span::styled(format!("  {l}"), Style::default().fg(color))))
                    .collect();
                if let Some(ChatBlock::Tool { output: o, .. }) = self
                    .blocks
                    .iter_mut()
                    .rev()
                    .find(|b| matches!(b, ChatBlock::Tool { id: bid, .. } if bid == id))
                {
                    o.extend(out);
                } else {
                    self.blocks.push(ChatBlock::Tool {
                        id: id.clone(),
                        header: Line::from(Span::styled(
                            "\u{25b8} (output)",
                            Style::default().fg(theme::accent()),
                        )),
                        output: out,
                        collapsed: true,
                    });
                }
                // Render tool-returned images inline after the text output.
                for url in images {
                    let (filename, rendered_img) = crate::image_render::build_image_block(url);
                    self.blocks.push(ChatBlock::Image {
                        filename,
                        rendered: rendered_img,
                    });
                }
            }
            SessionEvent::AgentSwitch(to) => {
                self.finalize_assistant();
                self.agent = to.clone();
                if to == "plan" {
                    self.plan_submitted = false;
                }
            }
            SessionEvent::ModelSwitch(m) => {
                self.finalize_assistant();
                // Strip a provider prefix defensively so the marker shows the
                // bare model id even if a stale/persisted event carries the
                // full "provider/model" string (issue #1).
                let bare = m.split_once('/').map(|(_, id)| id).unwrap_or(m);
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("[model] {bare}"),
                        Style::default().fg(theme::local_color()),
                    ))]));
            }
            SessionEvent::Compaction(c) => {
                self.finalize_compaction(c);
            }
            SessionEvent::Status(s) => self.status = s.clone(),
            SessionEvent::SubagentStart {
                id,
                kind,
                prompt,
                child_session_id,
            } => {
                self.subagents_running = self.subagents_running.saturating_add(1);
                self.subagents_total = self.subagents_total.saturating_add(1);
                self.finalize_assistant();
                // On the SECOND concurrent subagent, begin withholding the
                // parent's preamble assistant text (issue #5). It renders zero
                // lines until every sibling finishes, then reappears in one shot.
                if self.subagents_running == 2 && self.hidden_assistant_idx.is_none() {
                    self.hidden_assistant_idx = self
                        .blocks
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, b)| matches!(b, ChatBlock::Assistant { .. }))
                        .map(|(i, _)| i);
                }
                self.blocks.push(ChatBlock::Subagent {
                    id: id.clone(),
                    child_session_id: child_session_id.clone(),
                    kind: kind.clone(),
                    prompt: short(prompt, 90),
                    view: ChatView::default(),
                    done: false,
                    ok: false,
                    cancelled: false,
                    summary: String::new(),
                });
            }
            SessionEvent::SubagentChild { id, ev } => {
                if let Some(ChatBlock::Subagent { view, .. }) = self
                    .blocks
                    .iter_mut()
                    .rev()
                    .find(|b| matches!(b, ChatBlock::Subagent { id: bid, .. } if bid == id))
                {
                    view.apply(ev);
                }
            }
            SessionEvent::SubagentEnd {
                id,
                ok,
                cancelled,
                summary,
            } => {
                self.subagents_running = self.subagents_running.saturating_sub(1);
                self.finalize_assistant();
                // Mark done immediately — each subagent's status/summary should
                // surface as soon as it finishes, not be buffered behind siblings.
                self.mark_subagent_done(id, *ok, *cancelled, summary);
                if self.subagents_running == 0 {
                    self.hidden_assistant_idx = None;
                }
            }
            SessionEvent::Done => {
                self.subagents_running = 0;
                self.hidden_assistant_idx = None;
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Marker(vec![Line::from("")]));
            }
            SessionEvent::Error(e) => {
                self.subagents_running = 0;
                self.hidden_assistant_idx = None;
                self.finalize_assistant();
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("error: {e}"),
                        Style::default().fg(theme::err_color()),
                    ))]));
            }
            SessionEvent::PlanHandoff(plan) => {
                // Dedup: don't stack a second plan card on a replayed PlanHandoff.
                if self
                    .blocks
                    .iter()
                    .any(|b| matches!(b, ChatBlock::Plan { .. }))
                {
                    return;
                }
                self.finalize_assistant();
                let rendered = crate::markdown::render(plan);
                if !rendered.is_empty() {
                    self.blocks.push(ChatBlock::Plan {
                        rendered,
                        raw: plan.clone(),
                    });
                }
            }
            SessionEvent::TranscriptReset(_) => {}
            SessionEvent::QueueConsumed { .. } => {}
            SessionEvent::SteerConsumed { seq } => {
                // Steer promoted at turn boundary. The `steer:` marker is echoed
                // at admit time (app.rs), so here we only drop the consumed row
                // from the pending mirror.
                self.steer_items.retain(|(s, _)| s != seq);
            }
            SessionEvent::AutoPilot { phase, iteration } => {
                self.status = format!("autopilot: {:?} #{}", phase, iteration);
            }
        }
    }

    /// Record that a user requirement was delivered to the current agent.
    /// In plan mode this arms the plan→act handoff, so Shift+Tab collapses the
    /// planning transcript (only the final plan carries over). Every delivery
    /// path — Enter-submit, Tab-queue while running — must call this; a
    /// requirement given via the queue panel is still a requirement.
    pub fn note_requirement_submitted(&mut self) {
        if self.agent == "plan" {
            self.plan_submitted = true;
        }
    }

    /// Begin a new turn. The single owner of the turn-start invariant: any
    /// transient presentation status (e.g. an `[interrupted] ...` marker set on
    /// the previous turn) must be cleared so it does not leak into the status
    /// bar of the freshly-started turn. The transcript blocks are untouched.
    pub fn begin_turn(&mut self) {
        self.status.clear();
    }

    /// Push a non-streamed line and ensure the next TextDelta starts a new
    /// assistant block instead of merging into a prior one.
    pub fn push_marker(&mut self, line: Line<'static>) {
        self.finalize_assistant();
        self.blocks.push(ChatBlock::Marker(vec![line]));
    }

    /// Push several non-streamed lines as a single marker block and ensure the
    /// next TextDelta starts a fresh assistant block. Used by display-only
    /// commands (e.g. `/ps`) whose multi-line echo never reaches the model.
    pub fn push_marker_lines(&mut self, lines: Vec<Line<'static>>) {
        self.finalize_assistant();
        self.blocks.push(ChatBlock::Marker(lines));
    }

    /// Render the current assistant block's raw text as markdown (idempotent).
    /// Also seals a trailing unsealed Thinking block so its tokens are counted
    /// exactly once at the turn boundary (covers reasoning-only turns).
    pub fn finalize_assistant(&mut self) {
        // Reasoning → non-text transition: count a trailing unsealed Thinking
        // block once. Mutually exclusive with the Assistant branch below since
        // `last_mut()` is either a Thinking or an Assistant.
        if let Some(ChatBlock::Thinking { text, sealed, .. }) = self.blocks.last_mut() {
            if !*sealed {
                self.context_used += estimate(text) as u64;
                *sealed = true;
            }
        }
        // Assistant text finalization: render markdown + count once.
        if let Some(ChatBlock::Assistant {
            raw,
            rendered,
            done,
        }) = self.blocks.last_mut()
        {
            if !*done {
                self.context_used += estimate(raw) as u64;
                *rendered = crate::markdown::render(raw);
                *done = true;
            }
        }
    }

    /// Toggle collapse on the thinking block at `block_idx` (mouse click
    /// handler). No-op if the index is out of range or not a Thinking block.
    pub fn toggle_thinking_at(&mut self, block_idx: usize) {
        if let Some(ChatBlock::Thinking { collapsed, .. }) = self.blocks.get_mut(block_idx) {
            *collapsed = !*collapsed;
        }
    }

    /// Toggle collapse on the tool-output block at `block_idx` (mouse click
    /// handler). No-op if the index is out of range or not a Tool block.
    pub fn toggle_tool_at(&mut self, block_idx: usize) {
        if let Some(ChatBlock::Tool { collapsed, .. }) = self.blocks.get_mut(block_idx) {
            *collapsed = !*collapsed;
        }
    }

    /// Collapse every collapsible block (Thinking + Tool output) in this view.
    /// Bound to Ctrl+L: clears any expanded reasoning/tool-output blocks in one
    /// keystroke (also applied to a child subagent view before exiting it).
    pub fn collapse_all_collapsible(&mut self) {
        for block in &mut self.blocks {
            match block {
                ChatBlock::Thinking { collapsed, .. }
                | ChatBlock::Tool { collapsed, .. }
                | ChatBlock::Compaction { collapsed, .. } => {
                    *collapsed = true;
                }
                _ => {}
            }
        }
    }

    /// Accumulate estimated token counts for this view's OWN transcript only.
    /// Child subagent tokens are excluded — each child ChatView tracks its own
    /// subtree via its own `apply` (events route through `SubagentChild`).
    fn track_context(&mut self, ev: &SessionEvent) {
        // Note: TextDelta/ReasoningDelta are intentionally NOT counted here.
        // Counting per-delta made the status bar's ctx% indicator jump on
        // every token.
        // Instead they are counted once at turn boundaries via
        // `finalize_assistant` (and `ensure_assistant_open` for the
        // reasoning → text transition). The discrete events below are kept
        // immediate since they are low-frequency and not part of streaming.
        match ev {
            SessionEvent::ToolStart { input, .. } => {
                self.context_used += estimate(&input.to_string()) as u64;
            }
            SessionEvent::ToolEnd { output, .. } => {
                self.context_used += estimate(output) as u64;
            }
            SessionEvent::SubagentEnd { summary, .. } => {
                self.context_used += estimate(summary) as u64;
            }
            SessionEvent::Compaction(c) => {
                self.context_used = estimate(c) as u64;
            }
            SessionEvent::PlanHandoff(plan) => {
                self.context_used += estimate(plan) as u64;
            }
            _ => {}
        }
    }

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

    /// Return each Tool block's `(block_idx, header_line_idx)`. Mirrors
    /// `thinking_headers`; used to build click hit-rects for tool headers.
    pub fn tool_headers(&self) -> Vec<ToolHeader> {
        self.collect_headers().2
    }

    /// Single pass over all blocks computing the header line index of every
    /// Thinking / Subagent / Tool block, using the identical per-block line
    /// accounting that `flatten_with()` emits. Keeping the accounting in one
    /// place guarantees hit-rect indices stay aligned with the live render.
    fn collect_headers(
        &self,
    ) -> (
        Vec<ThinkingHeader>,
        Vec<SubagentHeader>,
        Vec<ToolHeader>,
        Vec<CompactionHeader>,
    ) {
        let mut think = Vec::new();
        let mut sub = Vec::new();
        let mut tool = Vec::new();
        let mut compaction = Vec::new();
        let mut line_idx = 0usize;
        for (block_idx, block) in self.blocks.iter().enumerate() {
            match block {
                ChatBlock::Marker(lines) => line_idx += lines.len(),
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
                    // Withheld preamble renders zero lines (issue #5); skip it
                    // so header line indices stay aligned with `flatten_with`.
                    if self.is_withheld(block_idx) {
                        continue;
                    }
                    // +1 for the "say:" header line emitted by flatten().
                    line_idx += 1;
                    line_idx += if *done {
                        rendered.len()
                    } else {
                        raw.split('\n').count()
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
                ChatBlock::Tool {
                    output, collapsed, ..
                } => {
                    tool.push(ToolHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    // Header always rendered; output + trailing blank only when expanded.
                    line_idx += 1 + if *collapsed { 0 } else { output.len() + 1 };
                }
                ChatBlock::Compaction { text, collapsed, .. } => {
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
                    line_idx += 1 + rendered.len() + 1;
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
        (think, sub, tool, compaction)
    }

    /// Flatten all blocks into a single `Vec<Line>` for rendering, using
    /// `anim_tick` only to advance the running-subagent spinner. Delegated to
    /// by `flatten()` (which passes `0`) for non-render callers (selection,
    /// scroll-counting, tests) — line counts are identical across tick values,
    /// so hit-rects and selection math stay aligned with the live render.
    pub fn flatten_with(&self, anim_tick: u32) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.blocks.len() * 2);
        for (block_idx, block) in self.blocks.iter().enumerate() {
            match block {
                ChatBlock::Marker(lines) => out.extend(lines.iter().cloned()),
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
                    // Withheld while multiple subagents run (issue #5): render
                    // zero lines so hit-rect/selection indices stay aligned.
                    if self.is_withheld(block_idx) {
                        continue;
                    }
                    // Visual header so assistant output has its own labelled region,
                    // mirroring the `user:` marker on user prompts.
                    out.push(Line::from(Span::styled(
                        "\u{276f} say:",
                        Style::default()
                            .fg(theme::ok_color())
                            .add_modifier(Modifier::BOLD),
                    )));
                    let indent = Span::raw("    ");
                    if *done {
                        for l in rendered.iter() {
                            let mut spans = vec![indent.clone()];
                            spans.extend(l.spans.iter().cloned());
                            out.push(Line::from(spans));
                        }
                    } else {
                        // Mirrors `flush_code` (markdown.rs): split the raw
                        // stream on `\n` and drop only the single trailing
                        // empty element produced by a terminating newline, so
                        // it does not render as an extra blank body line.
                        // Interior blank lines are preserved.
                        let mut rows: Vec<&str> = raw.split('\n').collect();
                        if rows.last().is_some_and(|s| s.is_empty()) {
                            rows.pop();
                        }
                        for l in rows {
                            let l = l.strip_suffix('\r').unwrap_or(l);
                            out.push(Line::from(vec![indent.clone(), Span::raw(l.to_string())]));
                        }
                    }
                }
                ChatBlock::Thinking { text, collapsed, .. } => {
                    out.extend(render_collapsible(
                        "\u{1f4ad}",
                        "Thinking",
                        text,
                        *collapsed,
                    ));
                }
                ChatBlock::Compaction { text, collapsed, .. } => {
                    out.extend(render_collapsible(
                        "\u{1f5dc}",
                        "Compaction",
                        text,
                        *collapsed,
                    ));
                }
                ChatBlock::Tool {
                    header,
                    output,
                    collapsed,
                    ..
                } => {
                    if *collapsed {
                        let n = output.len();
                        let mut spans = header.spans.clone();
                        if n > 0 {
                            spans.push(Span::styled(
                                format!(" [\u{2193} {n}]"),
                                Style::default().fg(theme::muted()),
                            ));
                        }
                        out.push(Line::from(spans));
                    } else {
                        let mut spans = header.spans.clone();
                        // Expanded: flip the header's leading prefix arrow
                        // from U+25B8 (right-pointing) to U+25BE (down-
                        // pointing) so the prefix mirrors the toggle state.
                        if let Some(first) = spans.first_mut() {
                            let flipped = match first.content.strip_prefix('\u{25b8}') {
                                Some(rest) => format!("\u{25be}{rest}"),
                                None => first.content.to_string(),
                            };
                            first.content = flipped.into();
                        }
                        spans.push(Span::styled(
                            " [\u{2191}]",
                            Style::default().fg(theme::muted()),
                        ));
                        out.push(Line::from(spans));
                        out.extend(output.iter().cloned());
                        out.push(Line::from(""));
                    }
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
                        let indent = Span::raw("    ");
                        for l in rendered.iter() {
                            let mut spans = vec![indent.clone()];
                            spans.extend(l.spans.iter().cloned());
                            out.push(Line::from(spans));
                        }
                    }
                    out.push(Line::from(""));
                }
                ChatBlock::Plan { rendered, .. } => {
                    out.push(Line::from(Span::styled(
                        "\u{2576}\u{2500} plan \u{2500}\u{2574}",
                        Style::default()
                            .fg(theme::warn_color())
                            .add_modifier(Modifier::BOLD),
                    )));
                    let indent = Span::raw("  ");
                    for l in rendered.iter() {
                        let mut spans = vec![indent.clone()];
                        spans.extend(l.spans.iter().cloned());
                        out.push(Line::from(spans));
                    }
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
                    ..
                } => {
                    let tool_count = view
                        .blocks
                        .iter()
                        .filter(|b| matches!(b, ChatBlock::Tool { .. }))
                        .count();
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
                            format!("{mark} {status_word}, {tool_count} tools"),
                            Style::default().fg(mark_color),
                        ),
                        Span::styled(" [\u{2192} view]", Style::default().fg(theme::muted())),
                    ];
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
        out
    }

    /// Non-animated flatten for callers that don't render (selection extract,
    /// scroll-counting, tests). Line counts match `flatten_with` exactly.
    pub fn flatten(&self) -> Vec<Line<'static>> {
        self.flatten_with(0)
    }

    /// Whether the block at `idx` is currently withheld from the rendered
    /// output — the parent's preamble assistant block while MULTIPLE
    /// subagents are in flight (issue #5). `flatten_with` and both header
    /// line-accounting functions consult this so hit-rects stay aligned with
    /// what's on screen.
    fn is_withheld(&self, idx: usize) -> bool {
        self.hidden_assistant_idx == Some(idx) && self.subagents_running >= 1
    }

    /// Mark the subagent block matching `id` as done. If no block exists
    /// (defensive), emit a fallback marker so the event stays visible.
    /// `cancelled` renders a distinct interrupted badge.
    fn mark_subagent_done(&mut self, id: &str, ok: bool, cancelled: bool, summary: &str) {
        if let Some(ChatBlock::Subagent {
            done,
            ok: bok,
            cancelled: bcan,
            summary: smry,
            ..
        }) = self
            .blocks
            .iter_mut()
            .rev()
            .find(|b| matches!(b, ChatBlock::Subagent { id: bid, .. } if bid == id))
        {
            *done = true;
            *bok = ok;
            *bcan = cancelled;
            *smry = summary.to_string();
        } else {
            let (mark, color) = if cancelled {
                ("\u{2298}", theme::muted())
            } else if ok {
                ("\u{2714}", theme::ok_color())
            } else {
                ("\u{2718}", theme::err_color())
            };
            self.blocks.push(ChatBlock::Marker(vec![Line::from(vec![
                Span::styled(format!("  {mark} subagent "), Style::default().fg(color)),
                Span::styled(short(summary, 110), Style::default().fg(theme::muted())),
            ])]));
        }
    }

    fn ensure_assistant_open(&mut self) {
        if !matches!(
            self.blocks.last(),
            Some(ChatBlock::Assistant { done: false, .. })
        ) {
            // Seal a trailing unsealed Thinking block so its tokens are counted
            // exactly once before it stops being the last block.
            if let Some(ChatBlock::Thinking { text, sealed, .. }) = self.blocks.last_mut() {
                if !*sealed {
                    self.context_used += estimate(text) as u64;
                    *sealed = true;
                }
            }
            self.blocks.push(ChatBlock::Assistant {
                raw: String::new(),
                rendered: Vec::new(),
                done: false,
            });
        }
    }

    fn ensure_thinking_open(&mut self) {
        if !matches!(
            self.blocks.last(),
            Some(ChatBlock::Thinking { sealed: false, .. })
        ) {
            self.blocks.push(ChatBlock::Thinking {
                text: String::new(),
                collapsed: true,
                sealed: false,
            });
        }
    }

    /// True if the last block is a collapsed Thinking block — i.e. the active
    /// reasoning stream is hidden, so per-delta re-renders can be skipped.
    pub fn last_thinking_collapsed(&self) -> bool {
        matches!(
            self.blocks.last(),
            Some(ChatBlock::Thinking {
                collapsed: true,
                ..
            })
        )
    }
}

#[cfg(test)]
#[path = "chat_tests/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "subagent_tests.rs"]
mod subagent_tests;
