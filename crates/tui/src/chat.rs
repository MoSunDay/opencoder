use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use opencode_llm::estimate;
use opencode_session::SessionEvent;

const TOOL_OUTPUT_LINES: usize = 6;

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
    Thinking { text: String, collapsed: bool },
    /// Tool invocation: header line + truncated output lines.
    Tool {
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
            SessionEvent::ToolStart { name, input, .. } => {
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Tool {
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
                output, is_error, ..
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
                if let Some(ChatBlock::Tool { output: o, .. }) = self.blocks.last_mut() {
                    o.extend(out);
                } else {
                    self.blocks.push(ChatBlock::Tool {
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
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("[switched to {to} mode]"),
                        Style::default().fg(Color::Magenta),
                    ))]));
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
                if let Some(block) = self
                    .blocks
                    .iter_mut()
                    .rev()
                    .find(|b| matches!(b, ChatBlock::Subagent { id: bid, .. } if bid == id))
                {
                    if let ChatBlock::Subagent {
                        done,
                        ok: bok,
                        summary: smry,
                        ..
                    } = block
                    {
                        *done = true;
                        *bok = *ok;
                        *smry = summary.clone();
                    }
                } else {
                    let mark = if *ok { "\u{2714}" } else { "\u{2718}" };
                    let color = if *ok { Color::Green } else { Color::Red };
                    self.blocks.push(ChatBlock::Marker(vec![Line::from(vec![
                        Span::styled(format!("  {mark} subagent "), Style::default().fg(color)),
                        Span::styled(short(summary, 110), Style::default().fg(Color::DarkGray)),
                    ])]));
                }
            }
            SessionEvent::Done => {
                self.subagents_running = 0;
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Marker(vec![Line::from("")]));
            }
            SessionEvent::Error(e) => {
                self.subagents_running = 0;
                self.finalize_assistant();
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("error: {e}"),
                        Style::default().fg(Color::Red),
                    ))]));
            }
            SessionEvent::TranscriptReset(_) => {}
            SessionEvent::QueueConsumed { .. } => {}
        }
    }

    /// Push a non-streamed line and ensure the next TextDelta starts a new
    /// assistant block instead of merging into a prior one.
    pub fn push_marker(&mut self, line: Line<'static>) {
        self.finalize_assistant();
        self.blocks.push(ChatBlock::Marker(vec![line]));
    }

    /// Render the current assistant block's raw text as markdown (idempotent).
    pub fn finalize_assistant(&mut self) {
        if let Some(ChatBlock::Assistant {
            raw,
            rendered,
            done,
        }) = self.blocks.last_mut()
        {
            if !*done {
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
        match ev {
            SessionEvent::TextDelta(t) | SessionEvent::ReasoningDelta(t) => {
                self.context_used += estimate(t) as u64;
            }
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
                    // +1 for the "say:" header line emitted by flatten().
                    line_idx += 1;
                    line_idx += if *done {
                        rendered.len()
                    } else {
                        raw.split('\n').count()
                    };
                }
                ChatBlock::Thinking { text, collapsed } => {
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
                    line_idx += 1;
                    line_idx += if *done {
                        rendered.len()
                    } else {
                        raw.split('\n').count()
                    };
                }
                ChatBlock::Thinking { text, collapsed } => {
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

    /// Flatten all blocks into a single `Vec<Line>` for rendering. This is
    /// called once per frame; markdown rendering happens only at finalization,
    /// so streaming stays O(text length) not O(parse + render).
    pub fn flatten(&self) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.blocks.len() * 2);
        for block in &self.blocks {
            match block {
                ChatBlock::Marker(lines) => out.extend(lines.iter().cloned()),
                ChatBlock::Assistant {
                    raw,
                    rendered,
                    done,
                } => {
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
                ChatBlock::Thinking { text, collapsed } => {
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
                ChatBlock::Tool { header, output } => {
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
                    // Status badge: colored dot/check/cross + word.
                    let (mark, mark_color, status_word) = if *done {
                        if *ok {
                            ("\u{2714}", Color::Green, "done")
                        } else {
                            ("\u{2718}", Color::Red, "failed")
                        }
                    } else {
                        ("\u{25cf}", Color::Yellow, "running")
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

    fn ensure_assistant_open(&mut self) {
        if !matches!(
            self.blocks.last(),
            Some(ChatBlock::Assistant { done: false, .. })
        ) {
            self.blocks.push(ChatBlock::Assistant {
                raw: String::new(),
                rendered: Vec::new(),
                done: false,
            });
        }
    }

    fn ensure_thinking_open(&mut self) {
        if !matches!(self.blocks.last(), Some(ChatBlock::Thinking { .. })) {
            self.blocks.push(ChatBlock::Thinking {
                text: String::new(),
                collapsed: true,
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

fn short(s: &str, n: usize) -> String {
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
mod tests {
    use super::*;

    #[test]
    fn text_delta_appends_to_assistant_block() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta("hello ".into()));
        v.apply(&SessionEvent::TextDelta("world".into()));
        assert!(block_text(&v).contains("hello"));
        assert!(block_text(&v).contains("world"));
    }

    #[test]
    fn reasoning_delta_creates_thinking_block() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::ReasoningDelta("analyzing".into()));
        let flat = v.flatten();
        // Collapsed by default: header shows "Thinking"
        assert!(flat
            .iter()
            .any(|l| { l.spans.iter().any(|s| s.content.contains("Thinking")) }));
        // Content hidden when collapsed
        assert!(!block_text(&v).contains("analyzing"));
        // Expand via block index and verify content
        v.toggle_thinking_at(0);
        assert!(block_text(&v).contains("analyzing"));
    }

    #[test]
    fn thinking_block_collapses() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::ReasoningDelta("line1\nline2\nline3".into()));
        // Collapsed by default: summary line only, content hidden
        let text = block_text(&v);
        assert!(text.contains("3 lines"));
        assert!(!text.contains("line1"));
        // Expand: should contain all 3 lines
        v.toggle_thinking_at(0);
        assert!(block_text(&v).contains("line1"));
        assert!(block_text(&v).contains("line3"));
        // Collapse again
        v.toggle_thinking_at(0);
        assert!(!block_text(&v).contains("line1"));
    }

    #[test]
    fn thinking_headers_match_flatten_line_indices() {
        let mut v = ChatView::default();
        // Two thinking blocks separated by an assistant block.
        v.apply(&SessionEvent::ReasoningDelta("think-a".into()));
        v.apply(&SessionEvent::TextDelta("hi".into()));
        v.apply(&SessionEvent::Done);
        v.apply(&SessionEvent::ReasoningDelta("think-b-1\nthink-b-2".into()));

        let flat = v.flatten();
        let headers = v.thinking_headers();
        assert_eq!(headers.len(), 2, "expected two thinking headers");
        // Each recorded header line must contain the "Thinking" header text.
        for h in &headers {
            let line = &flat[h.header_line_idx];
            assert!(
                line.spans.iter().any(|s| s.content.contains("Thinking")),
                "header_line_idx {} is not a Thinking header: {:?}",
                h.header_line_idx,
                line,
            );
        }
        // block_idx maps back to a Thinking block.
        for h in &headers {
            assert!(
                matches!(v.blocks[h.block_idx], ChatBlock::Thinking { .. }),
                "block_idx {} is not a Thinking block",
                h.block_idx,
            );
        }
        // Expanding the second block shifts nothing before it; first header
        // line index is unchanged.
        let first_before = headers[0].header_line_idx;
        v.toggle_thinking_at(headers[1].block_idx);
        let first_after = v.thinking_headers()[0].header_line_idx;
        assert_eq!(first_before, first_after);
    }

    #[test]
    fn toggle_thinking_at_toggles_specific_block() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::ReasoningDelta("first".into()));
        v.apply(&SessionEvent::TextDelta("between".into()));
        v.apply(&SessionEvent::Done);
        v.apply(&SessionEvent::ReasoningDelta("second".into()));

        let headers = v.thinking_headers();
        assert_eq!(headers.len(), 2);
        // Both collapsed initially.
        assert!(!block_text(&v).contains("first"));
        assert!(!block_text(&v).contains("second"));
        // Toggle only the first: its content shows, second stays hidden.
        v.toggle_thinking_at(headers[0].block_idx);
        assert!(block_text(&v).contains("first"));
        assert!(!block_text(&v).contains("second"));
        // Out-of-range / non-thinking index is a no-op.
        v.toggle_thinking_at(999);
        v.toggle_thinking_at(headers[0].block_idx + 1); // assistant block index
        assert!(block_text(&v).contains("first"));
    }

    #[test]
    fn done_renders_markdown() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta(
            "# Title\n\nSome **bold** text".into(),
        ));
        v.apply(&SessionEvent::Done);
        // After Done, the assistant block is finalized — check it has rendered
        for b in &v.blocks {
            if let ChatBlock::Assistant { done, .. } = b {
                assert!(*done, "assistant should be finalized after Done");
            }
        }
    }

    #[test]
    fn text_after_tool_starts_fresh_block() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta("result:".into()));
        v.apply(&SessionEvent::ToolStart {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: "t1".into(),
            name: "bash".into(),
            output: "file1".into(),
            is_error: false,
        });
        v.apply(&SessionEvent::TextDelta("done".into()));
        assert!(block_text(&v).contains("done"));
    }

    #[test]
    fn push_marker_separates_from_assistant() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta("streaming".into()));
        v.push_marker(Line::from("[queued] foo"));
        v.apply(&SessionEvent::TextDelta("more".into()));
        assert!(block_text(&v).contains("[queued] foo"));
        assert!(block_text(&v).contains("more"));
    }

    #[test]
    fn multiline_delta_splits_lines() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta("line1\nline2".into()));
        let flat = v.flatten();
        let texts: Vec<String> = flat
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
            .collect();
        // Assistant text is indented under the `say:` header
        assert!(texts.iter().any(|t| t.contains("line1")), "got {:?}", texts);
        assert!(texts.iter().any(|t| t.contains("line2")), "got {:?}", texts);
    }

    #[test]
    fn subagent_events_render() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta("parent asks subagent".into()));
        v.apply(&SessionEvent::SubagentStart {
            id: "s1".into(),
            kind: "explore".into(),
            prompt: "search".into(),
            child_session_id: "sub-1".into(),
        });
        assert!(block_text(&v).contains("subagent"));
        assert!(block_text(&v).contains("explore"));
        assert_eq!(v.subagents_total, 1);
        assert_eq!(v.subagents_running, 1);

        // Child events routed into the subagent block's view.
        let parent_ctx = v.context_used;
        assert!(parent_ctx > 0, "precondition: parent has its own tokens");
        v.apply(&SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::TextDelta("child output".into())),
        });
        assert_eq!(v.context_used, parent_ctx, "parent must not include child tokens");
        // No inline expansion — child output is always hidden in the parent.
        assert!(!block_text(&v).contains("child output"));
        // Child view itself contains the output (visible via ctx-switch).
        if let Some(ChatBlock::Subagent { view, .. }) = v
            .blocks
            .iter()
            .find(|b| matches!(b, ChatBlock::Subagent { .. }))
        {
            assert!(block_text(view).contains("child output"));
            // Child view tracks its own context.
            assert!(view.context_used > 0);
        } else {
            panic!("expected a Subagent block");
        }

        // SubagentEnd marks done and decrements running; summary shows on header.
        v.apply(&SessionEvent::SubagentEnd {
            id: "s1".into(),
            ok: true,
            summary: "found it".into(),
        });
        assert_eq!(v.subagents_running, 0);
        assert_eq!(v.subagents_total, 1);
        assert!(block_text(&v).contains("found it"));
        assert_eq!(v.context_used, parent_ctx + estimate("found it") as u64);
    }

    #[test]
    fn error_renders() {
        let mut v = ChatView::default();
        v.apply(&SessionEvent::Error("broke".into()));
        assert!(block_text(&v).contains("broke"));
    }

    #[test]
    fn paragraph_scroll_uses_wrapped_rows_and_pins_tail() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::{Paragraph, Widget, Wrap};

        let lines: Vec<Line> = vec![
            Line::from("AAAAAAAAAA"),
            Line::from("BBBBBBBBBB"),
            Line::from("CCCCCCCCCCEND"),
        ];
        let width = 10u16;
        let visible_h = 2u16;
        let total_rows = Paragraph::new(lines.clone())
            .wrap(Wrap { trim: false })
            .line_count(width);
        assert_eq!(total_rows, 4);
        let scroll_y = total_rows - visible_h as usize;
        let area = Rect::new(0, 0, width, visible_h);
        let mut buf = Buffer::empty(area);
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y as u16, 0))
            .render(area, &mut buf);
        let rs = |y: u16| -> String {
            (0..width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect()
        };
        assert!(rs(0).starts_with("CCCCCCCCCC"));
        assert!(rs(visible_h - 1).starts_with("END"));
    }
}
