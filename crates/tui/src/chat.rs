use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::terminal_text::{sanitize_line, sanitize_multiline, sanitize_single_line};
use crate::theme;

use opencoder_session::SessionEvent;

// Test modules under `chat_tests/` glob-import this scope and use `estimate`
// in token-accounting assertions (it previously lived here for
// `track_context`, now in `chat_context.rs`).
#[cfg(test)]
use opencoder_llm::estimate;

// ── Exact flattened header shapes (single source of truth) ────────────────
// These headers are emitted as single spans with exactly these contents, and
// copy-mode's structured cleaner (`crate::copy_mode::clean`) drops rows by
// matching the same constants — shape drift becomes a compile-time-visible
// shared change instead of a silent mis-classification.

/// `ChatBlock::User` header row.
pub(crate) const ROLE_USER_HEADER: &str = "\u{276f} User:";
/// `ChatBlock::Assistant` header row.
pub(crate) const ROLE_SAY_HEADER: &str = "\u{276f} Say:";
/// `ChatBlock::Plan` header row.
pub(crate) const PLAN_HEADER: &str = "\u{2576}\u{2500} plan \u{2500}\u{2574}";

/// Body lines of streaming assistant `raw`, mirroring `flatten_with`: drop the
/// single trailing empty element from a terminating newline (interior blanks kept).
/// Shared by `collect_headers` and `flatten_with` so they never diverge (A2/A3).
fn assistant_rows(raw: &str) -> Vec<&str> {
    let mut rows: Vec<&str> = raw.split('\n').collect();
    if rows.last().is_some_and(|s| s.is_empty()) {
        rows.pop();
    }
    rows
}

#[path = "chat_types.rs"]
mod types;
pub use types::*;

#[path = "chat_helpers.rs"]
mod helpers;
pub use helpers::block_text;
pub(crate) use helpers::{push_duration_span, short, summarize};

#[path = "compaction_block.rs"]
mod compaction_block;
#[path = "chat_context.rs"]
mod context;
#[path = "chat_headers.rs"]
mod headers;
#[path = "chat_sidecar.rs"]
pub(crate) mod sidecar;
#[path = "chat_stream.rs"]
mod stream;
pub(crate) use compaction_block::render_collapsible;

impl ChatView {
    pub fn apply(&mut self, ev: &SessionEvent) {
        self.track_context(ev);
        match ev {
            SessionEvent::LlmRoundStart { started_at_ms } => {
                self.llm_round_started_at_ms = Some(*started_at_ms);
                self.frozen_round_ms = None;
            }
            SessionEvent::LlmRoundEnd => {
                if let Some(anchor) = self.llm_round_started_at_ms.take() {
                    self.frozen_round_ms =
                        Some(((opencoder_core::message::now_ms() - anchor).max(0)) as u64);
                }
                self.finalize_assistant();
            }
            SessionEvent::LlmUsage { total_tokens, .. } => {
                self.tokens_total = self.tokens_total.saturating_add(*total_tokens);
                // Provider-truth context of this view's latest completed
                // round: the `total_tokens` the LLM actually returned
                // (input already includes the system prompt). Overwritten on
                // every usage-carrying round; never cleared on model switch
                // or compaction — the stale value stays until the next round
                // reports fresh usage.
                self.real_context_tokens = Some(*total_tokens);
            }
            SessionEvent::TextDelta(t) => {
                self.recover_round_anchor_if_missing();
                self.append_text_delta(&sanitize_multiline(t));
            }
            SessionEvent::ReasoningDelta(t) => {
                self.recover_round_anchor_if_missing();
                self.append_reasoning_delta(&sanitize_multiline(t));
            }
            // Stream the summary into an expanded block so it is visible while
            // the summarizing LLM call runs. The final `Compaction(summary)`
            // event (below) finalizes + collapses it.
            SessionEvent::CompactionDelta(t) => {
                self.open_compaction_streaming(&sanitize_multiline(t));
            }
            SessionEvent::ToolStart { id, name, input } => {
                if name == "task" {
                    return;
                }
                self.finalize_assistant();
                let call = ToolCall {
                    id: id.clone(),
                    header: Line::from(vec![
                        Span::styled(
                            format!("\u{25b8} {} ", sanitize_single_line(name)),
                            Style::default()
                                .fg(theme::accent())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(summarize(input), Style::default().fg(theme::muted())),
                    ]),
                    output: Vec::new(),
                    started_at_ms: Some(opencoder_core::message::now_ms()),
                    elapsed_ms: None,
                    expanded: false,
                };
                // Consecutive tool calls join the trailing group (keeping its
                // display state); any other block in between splits the run
                // into a new, Collapsed group.
                match self.blocks.last_mut() {
                    Some(ChatBlock::ToolGroup { calls, .. }) => calls.push(call),
                    _ => self.blocks.push(ChatBlock::ToolGroup {
                        calls: vec![call],
                        state: ToolGroupState::Collapsed,
                    }),
                }
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
                let clean_output = sanitize_multiline(output);
                let out: Vec<Line<'static>> = clean_output
                    .lines()
                    .take(TOOL_OUTPUT_LINES)
                    .map(|l| Line::from(Span::styled(format!("  {l}"), Style::default().fg(color))))
                    .collect();
                // Route by id: walk groups newest-first, calls newest-first,
                // so parallel calls each land in their own slot.
                let target = self.blocks.iter().enumerate().rev().find_map(|(gi, blk)| {
                    if let ChatBlock::ToolGroup { calls, .. } = blk {
                        calls.iter().rposition(|c| c.id == *id).map(|ci| (gi, ci))
                    } else {
                        None
                    }
                });
                match target {
                    Some((gi, ci)) => {
                        if let ChatBlock::ToolGroup { calls, .. } = &mut self.blocks[gi] {
                            let c = &mut calls[ci];
                            c.output.extend(out);
                            if let Some(started) = c.started_at_ms {
                                c.elapsed_ms = Some(
                                    ((opencoder_core::message::now_ms() - started).max(0)) as u64,
                                );
                            }
                        }
                    }
                    None => {
                        // Orphan ToolEnd (lost ToolStart): synthesize a
                        // finished single-call group so the output is kept.
                        self.blocks.push(ChatBlock::ToolGroup {
                            calls: vec![ToolCall {
                                id: id.clone(),
                                header: Line::from(Span::styled(
                                    "\u{25b8} (output)",
                                    Style::default().fg(theme::accent()),
                                )),
                                output: out,
                                started_at_ms: None,
                                elapsed_ms: Some(0),
                                expanded: false,
                            }],
                            state: ToolGroupState::Collapsed,
                        });
                    }
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
            SessionEvent::AgentSwitch(to) => self.fold_agent_switch(to),
            SessionEvent::ModelSwitch(m) => {
                self.finalize_assistant();
                // A different model/tokenizer invalidates the old
                // provider-truth context semantically, but the value is
                // kept (display-only) until the new model's first round
                // reports usage — there is no estimate fallback anymore.
                // Strip a provider prefix defensively so the marker shows the
                // bare model id even if a stale/persisted event carries the
                // full "provider/model" string (issue #1).
                let bare = m.split_once('/').map(|(_, id)| id).unwrap_or(m);
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("[model] {}", sanitize_single_line(bare)),
                        Style::default().fg(theme::local_color()),
                    ))]));
            }
            SessionEvent::Compaction(c) => {
                // The transcript was rewritten down to the summary. The
                // pre-compaction provider truth stays displayed (stale but
                // real) until the next round under the compacted context
                // reports fresh usage.
                self.finalize_compaction(&sanitize_multiline(c));
            }
            SessionEvent::Status(s) => self.status = sanitize_single_line(s).into_owned(),
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
                    kind: sanitize_single_line(kind).into_owned(),
                    prompt: short(prompt, 90),
                    view: ChatView {
                        llm_round_started_at_ms: Some(opencoder_core::message::now_ms()),
                        ..Default::default()
                    },
                    done: false,
                    ok: false,
                    cancelled: false,
                    summary: String::new(),
                    started_at_ms: opencoder_core::message::now_ms(),
                    elapsed_ms: None,
                });
            }
            SessionEvent::SubagentChild { id, ev } => {
                // Subagent spend is part of this view's lifetime cost: fold
                // the child round's tokens in here while the child view below
                // also keeps its own (focused) copy. The child's context is
                // NOT folded into `real_context_tokens` — it lives in a
                // separate window from this view's.
                if let SessionEvent::LlmUsage { total_tokens, .. } = ev.as_ref() {
                    self.tokens_total = self.tokens_total.saturating_add(*total_tokens);
                }
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
                self.llm_round_started_at_ms = None;
                self.frozen_round_ms = None;
                self.subagents_running = 0;
                self.hidden_assistant_idx = None;
                self.reconcile_orphaned_subagents();
                self.finalize_assistant();
                self.blocks.push(ChatBlock::Marker(vec![Line::from("")]));
            }
            SessionEvent::Error(e) => {
                self.llm_round_started_at_ms = None;
                self.frozen_round_ms = None;
                self.subagents_running = 0;
                self.hidden_assistant_idx = None;
                self.reconcile_orphaned_subagents();
                self.finalize_assistant();
                self.blocks
                    .push(ChatBlock::Marker(vec![Line::from(Span::styled(
                        format!("error: {}", sanitize_single_line(e)),
                        Style::default().fg(theme::err_color()),
                    ))]));
            }
            // Legacy persisted `plan_handoff` SSE events parse to `None` and
            // never reach the display layer; the surviving handoff card comes
            // from replaying `meta.handoff_plan` (see session_ui::replay).
            SessionEvent::TranscriptReset(_) => {
                // The view is rebuilt from the new message list via the
                // replay path, which reconstructs provider truth from the
                // persisted usage of the surviving messages.
            }
            SessionEvent::QueueConsumed { .. } => {}
            SessionEvent::SteerConsumed { seq, text } => {
                // Model-facing echo only: a compound control command's tail,
                // nothing for a bare command (applied inline, never
                // recorded). Legacy persisted events fall back to the local
                // mirror through the same normalization.
                let display = if !text.is_empty() {
                    opencoder_session::consumed_echo_text(text)
                } else {
                    self.steer_items
                        .iter()
                        .find(|(s, _)| s == seq)
                        .and_then(|(_, d)| opencoder_session::consumed_echo_text(d))
                };
                let display = display
                    .map(|d| sanitize_multiline(&d).into_owned())
                    .unwrap_or_default();
                if !display.is_empty() {
                    self.blocks.push(ChatBlock::User {
                        rendered: crate::markdown::render(&display),
                    });
                    self.push_marker(Line::from(""));
                }
                self.steer_items.retain(|(s, _)| s != seq);
            }
            SessionEvent::AutoPilot { phase, iteration } => {
                self.status = format!("autopilot: {:?} #{}", phase, iteration);
            }
            // Sidecar frames fold into their dedicated block (see
            // `sidecar::fold_sidecar`): Start pushes + auto-focuses the block,
            // Child routes into the block's nested view, Turn finalizes it.
            // Bare `LlmUsage` is NOT a sidecar frame — it already took the
            // parent arm above, which is exactly how the sidecar's cost is
            // accounted to the main task (tokens_total).
            SessionEvent::SidecarStart { .. }
            | SessionEvent::SidecarChild { .. }
            | SessionEvent::SidecarTurn { .. } => {
                sidecar::fold_sidecar(self, ev);
            }
        }
    }

    /// Fold an agent switch into the view state: finalize any open assistant
    /// block and reflect the new agent.
    ///
    /// Split out of the `AgentSwitch` event arm so other paths can fold the
    /// switch synchronously at flip time.
    pub fn fold_agent_switch(&mut self, to: &str) {
        self.finalize_assistant();
        self.agent = sanitize_single_line(to).into_owned();
    }

    /// Begin a new turn. The single owner of the turn-start invariant: any
    /// transient presentation status (e.g. an `[interrupted] ...` marker set on
    /// the previous turn) must be cleared so it does not leak into the status
    /// bar of the freshly-started turn. The transcript blocks are untouched.
    pub fn begin_turn(&mut self) {
        self.submitted = true;
        self.status.clear();
        self.turn_block_start = self.blocks.len();
        self.llm_round_started_at_ms = Some(opencoder_core::message::now_ms());
        self.frozen_round_ms = None;
    }

    /// Push a non-streamed line and ensure the next TextDelta starts a new
    /// assistant block instead of merging into a prior one.
    pub fn push_marker(&mut self, line: Line<'static>) {
        self.finalize_assistant();
        self.blocks
            .push(ChatBlock::Marker(vec![sanitize_line(line)]));
    }

    /// Push several non-streamed lines as a single marker block and ensure the
    /// next TextDelta starts a fresh assistant block. Used by display-only
    /// commands (e.g. `/ps`) whose multi-line echo never reaches the model.
    pub fn push_marker_lines(&mut self, lines: Vec<Line<'static>>) {
        self.finalize_assistant();
        self.blocks.push(ChatBlock::Marker(
            lines.into_iter().map(sanitize_line).collect(),
        ));
    }

    /// Toggle collapse on the thinking block at `block_idx` (mouse click
    /// handler). No-op if the index is out of range or not a Thinking block.
    pub fn toggle_thinking_at(&mut self, block_idx: usize) {
        if let Some(ChatBlock::Thinking { collapsed, .. }) = self.blocks.get_mut(block_idx) {
            *collapsed = !*collapsed;
        }
    }

    /// Cycle the tool group at `block_idx` through its three display states:
    /// Collapsed → List → Results → Collapsed (mouse click handler). No-op if
    /// the index is out of range or not a ToolGroup block.
    pub fn cycle_tool_group_at(&mut self, block_idx: usize) {
        if let Some(ChatBlock::ToolGroup { state, .. }) = self.blocks.get_mut(block_idx) {
            *state = match *state {
                ToolGroupState::Collapsed => ToolGroupState::List,
                ToolGroupState::List => ToolGroupState::Results,
                ToolGroupState::Results => ToolGroupState::Collapsed,
            };
        }
    }

    /// Toggle the expanded output of the single call at `call_idx` inside the
    /// ToolGroup at `block_idx` (mouse click handler on a call header row).
    /// Only meaningful in the `List` state — `Collapsed` renders no call rows
    /// and `Results` shows every output regardless — so the flag is left
    /// untouched there. No-op if either index is out of range.
    pub fn toggle_tool_call_at(&mut self, block_idx: usize, call_idx: usize) {
        if let Some(ChatBlock::ToolGroup { calls, state }) = self.blocks.get_mut(block_idx) {
            if matches!(state, ToolGroupState::List) {
                if let Some(c) = calls.get_mut(call_idx) {
                    c.expanded = !c.expanded;
                }
            }
        }
    }

    /// Collapse every collapsible block (Thinking + Compaction + ToolGroup)
    /// in this view. Bound to Ctrl+L: clears any expanded reasoning blocks and
    /// resets every tool group to Collapsed in one keystroke (also applied to
    /// a child subagent view before exiting it).
    pub fn collapse_all_collapsible(&mut self) {
        for block in &mut self.blocks {
            match block {
                ChatBlock::Thinking { collapsed, .. } | ChatBlock::Compaction { collapsed, .. } => {
                    *collapsed = true;
                }
                ChatBlock::ToolGroup { calls, state } => {
                    *state = ToolGroupState::Collapsed;
                    for c in calls.iter_mut() {
                        c.expanded = false;
                    }
                }
                _ => {}
            }
        }
    }

    /// Flatten all blocks into a single `Vec<Line>` for rendering, using
    /// `anim_tick` only to advance the running-subagent spinner. Delegated to
    /// by `flatten()` (which passes `0`) for non-render callers (selection,
    /// scroll-counting, tests) — line counts are identical across tick values,
    /// so hit-rects and selection math stay aligned with the live render.
    pub fn flatten_with(&self, anim_tick: u32, now_ms: i64) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.blocks.len() * 2);
        for (block_idx, block) in self.blocks.iter().enumerate() {
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
                    // Withheld while multiple subagents run (issue #5): render
                    // zero lines so hit-rect/selection indices stay aligned.
                    if self.is_withheld(block_idx) {
                        continue;
                    }
                    // Visual header so assistant output has its own labelled region,
                    // mirroring the `user:` marker on user prompts.
                    out.push(Line::from(Span::styled(
                        ROLE_SAY_HEADER,
                        Style::default()
                            .fg(theme::ok_color())
                            .add_modifier(Modifier::BOLD),
                    )));
                    let indent = Span::raw("    ");
                    if *done {
                        out.extend(types::indented(rendered, 4));
                    } else {
                        // Mirrors `flush_code` (markdown.rs): split the raw
                        // stream on `\n` and drop only the single trailing
                        // empty element produced by a terminating newline, so
                        // it does not render as an extra blank body line.
                        // Interior blank lines are preserved. Shared with
                        // `collect_headers` so the two can never diverge.
                        let rows = assistant_rows(raw);
                        for l in rows {
                            let l = l.strip_suffix('\r').unwrap_or(l);
                            out.push(Line::from(vec![indent.clone(), Span::raw(l.to_string())]));
                        }
                    }
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
                ChatBlock::ToolGroup { calls, state } => {
                    let n = calls.len();
                    // Group line: `▸ N function calls` (arrow flips to ▾ once
                    // expanded) + a live spinner hint while any call in the
                    // group is still running.
                    let arrow = if matches!(state, ToolGroupState::Collapsed) {
                        "\u{25b8}"
                    } else {
                        "\u{25be}"
                    };
                    let mut spans = vec![Span::styled(
                        format!(
                            "{arrow} {n} function call{} ",
                            if n == 1 { "" } else { "s" }
                        ),
                        Style::default()
                            .fg(theme::accent())
                            .add_modifier(Modifier::BOLD),
                    )];
                    if calls.iter().any(|c| c.elapsed_ms.is_none()) {
                        spans.push(Span::styled(
                            format!("{} running ", SPINNER[(anim_tick as usize) % SPINNER.len()]),
                            Style::default().fg(theme::warn_color()),
                        ));
                    }
                    match state {
                        ToolGroupState::Collapsed => {
                            out.push(Line::from(spans));
                        }
                        ToolGroupState::List => {
                            spans.push(Span::styled(
                                "[\u{2193}]",
                                Style::default().fg(theme::muted()),
                            ));
                            out.push(Line::from(spans));
                            for c in calls {
                                out.extend(types::indented(std::slice::from_ref(&c.header), 2));
                                // Per-call expansion: only the toggled call
                                // shows its output in the List state.
                                if c.expanded {
                                    out.extend(c.output.iter().cloned());
                                    out.push(Line::from(""));
                                }
                            }
                            out.push(Line::from(""));
                        }
                        ToolGroupState::Results => {
                            spans.push(Span::styled(
                                "[\u{2191}]",
                                Style::default().fg(theme::muted()),
                            ));
                            out.push(Line::from(spans));
                            for c in calls {
                                out.extend(types::indented(std::slice::from_ref(&c.header), 2));
                                out.extend(c.output.iter().cloned());
                                out.push(Line::from(""));
                            }
                        }
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
                    let tool_count = view
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            ChatBlock::ToolGroup { calls, .. } => Some(calls.len()),
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
                            format!("{mark} {status_word}, {tool_count} tools"),
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
                // ZERO rows in the main transcript: the sidecar bypass Q/A
                // leaves no trace there. While focused, `compute_display`
                // swaps the whole body for the block's nested view; while
                // unfocused the block is simply invisible (`sidecar::purge`
                // removes it on exit anyway).
                ChatBlock::Sidecar { .. } => {}
            }
        }
        out
    }

    /// Non-animated flatten for callers that don't render (selection extract,
    /// scroll-counting, tests). Line counts match `flatten_with` exactly.
    pub fn flatten(&self) -> Vec<Line<'static>> {
        self.flatten_with(0, opencoder_core::message::now_ms())
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
            view,
            started_at_ms,
            elapsed_ms,
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
            *smry = sanitize_multiline(summary).into_owned();
            view.llm_round_started_at_ms = None;
            view.frozen_round_ms = None;
            // Leftover child steer rows (steers queued while the child was
            // running but never claimed) would otherwise sit on the pending
            // panel forever — clear them with the block.
            view.steer_items.clear();
            *elapsed_ms =
                Some(((opencoder_core::message::now_ms() - *started_at_ms).max(0)) as u64);
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

    /// Self-heal a missing round anchor: if `LlmRoundStart` was dropped by the
    /// saturated `forward_event` channel, the first `TextDelta`/`ReasoningDelta`
    /// re-anchors it (only when `None`; recursive for child views too).
    fn recover_round_anchor_if_missing(&mut self) {
        if self.llm_round_started_at_ms.is_none() {
            self.llm_round_started_at_ms = Some(opencoder_core::message::now_ms());
            self.frozen_round_ms = None;
        }
    }
}

#[cfg(test)]
#[path = "chat_tests/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "subagent_tests.rs"]
mod subagent_tests;
