use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use opencoder_llm::estimate;
use opencoder_session::SessionEvent;

const TOOL_OUTPUT_LINES: usize = 6;

/// Braille spinner frames shown next to a running subagent header. Matches the
/// status-bar spinner in `render.rs` so the UI has one consistent motion.
const SPINNER: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

/// A single visual block in the transcript. Replaces the flat `Vec<Line>`
/// model so we can have collapsible thinking blocks, streaming-vs-rendered
/// assistant text, and tool blocks with structured output.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatBlock {
    /// User prompt, queued/steer marker, system notice — plain styled lines.
    Marker(Vec<Line<'static>>),
    /// Assistant text output. While streaming (`done == false`) the raw text is
    /// shown as plain lines for low latency. On turn completion the text is
    /// rendered as markdown exactly once, then `done` flips to `true`.
    Assistant {
        raw: String,
        rendered: Vec<Line<'static>>,
        done: bool,
    },
    /// Collapsible reasoning/thinking block with dimmed italic styling.
    Thinking {
        text: String,
        collapsed: bool,
        sealed: bool,
    },
    /// Tool invocation: header line + truncated output lines.
    Tool {
        id: String,
        header: Line<'static>,
        output: Vec<Line<'static>>,
    },
    /// Foldable subagent block. Clicking the header enters the subagent's
    /// perspective (ctx-switch) showing its child `view` as the full body plus
    /// its own context stats. The header always renders as a single clickable
    /// line with running/done/failed status — no inline expansion.
    Subagent {
        id: String,
        child_session_id: String,
        kind: String,
        prompt: String,
        view: ChatView,
        done: bool,
        ok: bool,
        summary: String,
    },
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ChatView {
    pub blocks: Vec<ChatBlock>,
    pub agent: String,
    pub status: String,
    /// Number of subagents currently in flight (SubagentStart seen, no matching
    /// SubagentEnd yet). Surfaced in the status bar as a live "running" badge so
    /// concurrent dispatch is visible.
    pub subagents_running: u32,
    /// Total subagents dispatched this session (running + completed).
    pub subagents_total: u32,
    /// Estimated tokens consumed by this view's own transcript (excludes
    /// child subagent tokens, which live on the child ChatView). Used to
    /// show context stats when viewing a subagent's perspective.
    pub context_used: u64,
    /// Index of the parent's assistant block whose content is withheld while
    /// MULTIPLE subagents are in flight (see issue #5). The block renders zero
    /// lines in `flatten_with` and is excluded from header line-accounting so
    /// hit-rects stay aligned. Cleared once all subagents finish (the content
    /// then appears in one shot).
    pub hidden_assistant_idx: Option<usize>,
    /// Buffered `(id, ok, summary)` for `SubagentEnd` events that arrived while
    /// other subagents were still running. Applied in a single batch when the
    /// last sibling finishes, so completion summaries never pop in one-by-one.
    pub pending_subagent_ends: Vec<(String, bool, String)>,
}

/// Locates a `Thinking` block's header line for mouse hit-testing.
/// `header_line_idx` is the index within `ChatView::flatten()` of the block's
/// header line; `block_idx` is its index in `ChatView::blocks`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThinkingHeader {
    pub block_idx: usize,
    pub header_line_idx: usize,
}

/// Locates a `Subagent` block's header line for mouse hit-testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubagentHeader {
    pub block_idx: usize,
    pub header_line_idx: usize,
}

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
            SessionEvent::ToolStart { id, name, input } => {
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Tool {
                    id: id.clone(),
                    header: Line::from(vec![
                        Span::styled(
                            format!("\u{25b8} {name} "),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(summarize(input), Style::default().fg(Color::DarkGray)),
                    ]),
                    output: Vec::new(),
                });
            }
            SessionEvent::ToolEnd {
                id,
                output,
                is_error,
                ..
            } => {
                self.finalize_assistant();
                let color = if *is_error {
                    Color::Red
                } else {
                    Color::DarkGray
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
                            Style::default().fg(Color::Cyan),
                        )),
                        output: out,
                    });
                }
            }
            SessionEvent::AgentSwitch(to) => {
                self.finalize_assistant();
                self.agent = to.clone();
            }
            SessionEvent::Compaction(c) => {
                self.finalize_assistant();
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("[context compacted] {}", short(c, 100)),
                        Style::default().fg(Color::Yellow),
                    ))]));
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
            SessionEvent::SubagentEnd { id, ok, summary } => {
                self.subagents_running = self.subagents_running.saturating_sub(1);
                self.finalize_assistant();
                if self.subagents_running > 0 {
                    // Other siblings still running — buffer so all completion
                    // summaries surface together when the last one finishes
                    // (issue #5), instead of popping in one-by-one.
                    self.pending_subagent_ends
                        .push((id.clone(), *ok, summary.clone()));
                } else {
                    // Last (or only) sibling done — flush buffered ends in
                    // arrival order, apply this one, then reveal the preamble.
                    self.flush_pending_subagent_ends();
                    self.mark_subagent_done(id, *ok, summary);
                    self.hidden_assistant_idx = None;
                }
            }
            SessionEvent::Done => {
                self.subagents_running = 0;
                self.flush_pending_subagent_ends();
                self.hidden_assistant_idx = None;
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Marker(vec![Line::from("")]));
            }
            SessionEvent::Error(e) => {
                self.subagents_running = 0;
                self.flush_pending_subagent_ends();
                self.hidden_assistant_idx = None;
                self.finalize_assistant();
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("error: {e}"),
                        Style::default().fg(Color::Red),
                    ))]));
            }
            SessionEvent::TranscriptReset(_) => {}
            SessionEvent::QueueConsumed { .. } => {}
            SessionEvent::SteerConsumed { .. } => {}
        }
    }

    /// Push a non-streamed line and ensure the next TextDelta starts a new
    /// assistant block instead of merging into a prior one.
    pub fn push_marker(&mut self, line: Line<'static>) {
        self.finalize_assistant();
        self.blocks.push(ChatBlock::Marker(vec![line]));
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

    /// Accumulate estimated token counts for this view's OWN transcript only.
    /// Child subagent tokens are excluded — each child ChatView tracks its own
    /// subtree via its own `apply` (events route through `SubagentChild`).
    fn track_context(&mut self, ev: &SessionEvent) {
        // Note: TextDelta/ReasoningDelta are intentionally NOT counted here.
        // Counting per-delta made the bottom ctx% bar jump on every token.
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
            _ => {}
        }
    }

    /// Return each Thinking block's `(block_idx, header_line_idx)`, where
    /// `header_line_idx` is the index in `flatten()` of its header line. Walks
    /// the blocks with the same per-block line accounting `flatten()` uses, so
    /// the indices stay in sync with what is rendered. Used by `render_body`
    /// to build click hit-rects.
    pub fn thinking_headers(&self) -> Vec<ThinkingHeader> {
        let mut out = Vec::new();
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
                    out.push(ThinkingHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    // Header line always emitted; content lines only when expanded.
                    line_idx += 1;
                    if !collapsed {
                        line_idx += text.lines().count();
                    }
                }
                ChatBlock::Tool { output, .. } => {
                    // header line + output lines + trailing blank line.
                    line_idx += 1 + output.len() + 1;
                }
                ChatBlock::Subagent { .. } => {
                    line_idx += 1; // header only — no inline expansion
                }
            }
        }
        out
    }

    /// Return each Subagent block's `(block_idx, header_line_idx)` for
    /// mouse hit-testing, using the same line accounting as `flatten()`.
    pub fn subagent_headers(&self) -> Vec<SubagentHeader> {
        let mut out = Vec::new();
        let mut line_idx = 0usize;
        for (block_idx, block) in self.blocks.iter().enumerate() {
            match block {
                ChatBlock::Marker(lines) => line_idx += lines.len(),
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
                    if self.is_withheld(block_idx) {
                        continue;
                    }
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
                    line_idx += 1;
                    if !collapsed {
                        line_idx += text.lines().count();
                    }
                }
                ChatBlock::Tool { output, .. } => {
                    line_idx += 1 + output.len() + 1;
                }
                ChatBlock::Subagent { .. } => {
                    out.push(SubagentHeader {
                        block_idx,
                        header_line_idx: line_idx,
                    });
                    line_idx += 1; // header only — no inline expansion
                }
            }
        }
        out
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
                        "say:",
                        Style::default()
                            .fg(Color::Green)
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
                        for l in raw.split('\n') {
                            out.push(Line::from(vec![indent.clone(), Span::raw(l.to_string())]));
                        }
                    }
                }
                ChatBlock::Thinking {
                    text, collapsed, ..
                } => {
                    let count = text.lines().count().max(1);
                    if *collapsed {
                        out.push(Line::from(Span::styled(
                            format!("\u{1f4ad} Thinking ({count} lines) [\u{2193} expand]"),
                            Style::default().fg(Color::DarkGray),
                        )));
                    } else {
                        out.push(Line::from(Span::styled(
                            "\u{1f4ad} Thinking [\u{2191} collapse]",
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
                        )));
                        for l in text.lines() {
                            out.push(Line::from(Span::styled(
                                format!("  {l}"),
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            )));
                        }
                    }
                }
                ChatBlock::Tool { header, output, .. } => {
                    out.push(header.clone());
                    out.extend(output.iter().cloned());
                    out.push(Line::from(""));
                }
                ChatBlock::Subagent {
                    kind,
                    prompt,
                    view,
                    done,
                    ok,
                    summary,
                    ..
                } => {
                    let tool_count = view
                        .blocks
                        .iter()
                        .filter(|b| matches!(b, ChatBlock::Tool { .. }))
                        .count();
                    // Status badge: animated spinner/check/cross + word. The
                    // running spinner uses the live anim_tick for motion.
                    let (mark, mark_color, status_word) = if *done {
                        if *ok {
                            ("\u{2714}", Color::Green, "done")
                        } else {
                            ("\u{2718}", Color::Red, "failed")
                        }
                    } else {
                        (
                            SPINNER[(anim_tick as usize) % SPINNER.len()],
                            Color::Yellow,
                            "running",
                        )
                    };
                    let mut spans = vec![
                        Span::styled(
                            "\u{2937} subagent ",
                            Style::default()
                                .fg(Color::Blue)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("[{kind}] "), Style::default().fg(Color::Cyan)),
                        Span::styled(prompt.clone(), Style::default().fg(Color::DarkGray)),
                        Span::raw(" "),
                        Span::styled(
                            format!("{mark} {status_word}, {tool_count} tools"),
                            Style::default().fg(mark_color),
                        ),
                        Span::styled(" [\u{2192} view]", Style::default().fg(Color::DarkGray)),
                    ];
                    if *done && !summary.is_empty() {
                        spans.push(Span::styled(
                            format!("  {summary}"),
                            Style::default().fg(if *ok { Color::DarkGray } else { Color::Red }),
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
    fn mark_subagent_done(&mut self, id: &str, ok: bool, summary: &str) {
        if let Some(ChatBlock::Subagent {
            done,
            ok: bok,
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
            *smry = summary.to_string();
        } else {
            let mark = if ok { "\u{2714}" } else { "\u{2718}" };
            let color = if ok { Color::Green } else { Color::Red };
            self.blocks.push(ChatBlock::Marker(vec![Line::from(vec![
                Span::styled(format!("  {mark} subagent "), Style::default().fg(color)),
                Span::styled(short(summary, 110), Style::default().fg(Color::DarkGray)),
            ])]));
        }
    }

    /// Apply all buffered subagent completions in arrival order. Called when
    /// the last sibling finishes, or on Done/Error as a safety flush.
    fn flush_pending_subagent_ends(&mut self) {
        let drained = std::mem::take(&mut self.pending_subagent_ends);
        for (id, ok, summary) in drained {
            self.mark_subagent_done(&id, ok, &summary);
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
}

fn summarize(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(m) => {
            for k in ["command", "path", "description", "pattern", "prompt"] {
                if let Some(s) = m.get(k).and_then(|v| v.as_str()) {
                    return short(s, 80);
                }
            }
            short(&serde_json::to_string(input).unwrap_or_default(), 80)
        }
        o => short(&serde_json::to_string(o).unwrap_or_default(), 80),
    }
}

pub(crate) fn short(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        format!("{}...", t.chars().take(n).collect::<String>())
    }
}

/// Read the concatenated text content of all blocks (for testing).
pub fn block_text(view: &ChatView) -> String {
    view.flatten()
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "subagent_tests.rs"]
mod subagent_tests;
