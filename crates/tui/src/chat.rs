use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::terminal_text::{sanitize_line, sanitize_multiline, sanitize_single_line};
use crate::theme;

use opencoder_session::SessionEvent;

// Test modules under `chat_tests/` glob-import this scope and use `estimate`
// in token-accounting assertions.
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
/// Header row above an open step's folded thinking (inside a `StepGroup`),
/// and of the standalone expanded Thinking block.
pub(crate) const STEP_THINKING_HEADER: &str = "\u{1f4ad} Thinking";
/// `ChatBlock::Plan` header row.
pub(crate) const PLAN_HEADER: &str = "\u{2576}\u{2500} plan \u{2500}\u{2574}";

/// Body lines of streaming assistant `raw`, mirroring `flatten_with`: drop the
/// single trailing empty element from a terminating newline (interior blanks kept).
/// Shared by `collect_headers` and `flatten_with` so they never diverge (A2/A3).
pub(crate) fn assistant_rows(raw: &str) -> Vec<&str> {
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
pub(crate) use helpers::{push_duration_span, short, summarize, tool_output_lines};

#[path = "chat_step_render.rs"]
mod step_render;
pub(crate) use step_render::SayHeader;
#[path = "chat_steps.rs"]
mod steps;

#[path = "compaction_block.rs"]
mod compaction_block;
#[path = "chat_context.rs"]
mod context;
#[path = "chat_flatten.rs"]
mod flatten;
#[path = "chat_headers.rs"]
mod headers;
#[path = "chat_sidecar.rs"]
pub(crate) mod sidecar;
#[path = "chat_stream.rs"]
mod stream;
pub(crate) use compaction_block::render_collapsible;
pub(crate) use steps::{coalesce_steps, single_step_group, StepTarget};

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
            // event replaces the text without changing disclosure state.
            SessionEvent::CompactionDelta(t) => {
                self.open_compaction_streaming(&sanitize_multiline(t));
            }
            SessionEvent::ToolStart { id, name, input } => {
                if name == "task" {
                    return;
                }
                self.finalize_assistant();
                // The round's pending Thinking blocks become this round's
                // step-thinking (rendered markdown, step-local, out of the
                // main flow). The backwards walk crosses Assistant blocks
                // (the turn's own speech) but stops at any other block, so
                // the previous user segment's thinking is never absorbed.
                let thinking = steps::absorb_pending_thinking(&mut self.blocks);
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
                // Every non-task call in this admitted user turn joins its
                // ONE canonical group; Say and presentation blocks are not
                // structural boundaries. Calls keep accumulating in the
                // current Step until a later Thinking run opens the next.
                steps::merge_turn_call(&mut self.blocks, self.turn_block_start, thinking, call);
                steps::set_turn_progress(&mut self.blocks, self.turn_block_start, true);
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
                // Shared capture: trailing blank lines are dropped so the
                // structural blank after the expanded output stays the only one.
                let out = tool_output_lines(output, color);
                // Route by id: walk groups newest-first, steps newest-first,
                // calls newest-first, so parallel calls each land in their
                // own slot.
                let target = self.blocks.iter().enumerate().rev().find_map(|(gi, blk)| {
                    if let ChatBlock::StepGroup { steps, .. } = blk {
                        steps
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(si, s)| {
                                s.calls.iter().rposition(|c| c.id == *id).map(|ci| (si, ci))
                            })
                            .map(|(si, ci)| (gi, si, ci))
                    } else {
                        None
                    }
                });
                match target {
                    Some((gi, si, ci)) => {
                        if let ChatBlock::StepGroup { steps, .. } = &mut self.blocks[gi] {
                            let c = &mut steps[si].calls[ci];
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
                        // finished single-call step so the output is kept,
                        // folded into the trailing group when one exists —
                        // the same fold replay's `coalesce_steps` applies to
                        // adjacent groups — so the transcript renders
                        // identically before and after resume.
                        let call = ToolCall {
                            id: id.clone(),
                            header: Line::from(Span::styled(
                                "\u{25b8} (output)",
                                Style::default().fg(theme::accent()),
                            )),
                            output: out,
                            started_at_ms: None,
                            elapsed_ms: Some(0),
                            expanded: false,
                        };
                        steps::merge_turn_call(
                            &mut self.blocks,
                            self.turn_block_start,
                            Vec::new(),
                            call,
                        );
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
                // The task-tool round streams no absorbable ToolStart: flush
                // its pending Thinking into the ladder (never outside it).
                self.flush_pending_thinking();
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
            }
            SessionEvent::Done => {
                self.llm_round_started_at_ms = None;
                self.frozen_round_ms = None;
                self.subagents_running = 0;
                // Turn complete: its echo must never resurface on a later
                // rebuild (a bare `/act_clear_context` mid-run would
                // otherwise resurrect the previous turn's prompt).
                self.pending_turn_echo = None;
                self.reconcile_orphaned_subagents();
                self.finalize_assistant();
                steps::set_turn_progress(&mut self.blocks, self.turn_block_start, false);
                // A round with no tool call (pure-text turn, or the turn's
                // final Say round) folds its pending Thinking into a call-less
                // step — thinking never survives outside the ladder.
                self.flush_pending_thinking();
                // Exactly one blank after the turn (User:-block parity): a
                // tool-final turn ends on its ladder's own trailing blank, so
                // the boundary marker must not stack a second one.
                if !self.last_block_ends_blank() {
                    self.blocks.push(ChatBlock::Marker(vec![Line::from("")]));
                }
            }
            SessionEvent::Error(e) => {
                self.llm_round_started_at_ms = None;
                self.frozen_round_ms = None;
                self.subagents_running = 0;
                self.pending_turn_echo = None;
                self.reconcile_orphaned_subagents();
                self.finalize_assistant();
                steps::set_turn_progress(&mut self.blocks, self.turn_block_start, false);
                self.flush_pending_thinking();
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
                    // Segment boundary: the pre-boundary pending Thinking can
                    // never be absorbed by a later ToolStart (the walk stops
                    // here) — fold it into the ladder before the echo lands.
                    self.flush_pending_thinking();
                    self.blocks.push(ChatBlock::User {
                        rendered: crate::markdown::render(&display),
                    });
                    self.push_marker(Line::from(""));
                    // Remember the echo across a TranscriptReset rebuild (a
                    // steered `/act_clear_context <tail>` resets the view
                    // right after this event, before the tail is recorded).
                    self.pending_turn_echo = Some(display.clone());
                    // The echoed steer is a NEW user input: per the Turn
                    // contract (1 turn = n steps + say) the rounds it
                    // triggers own a FRESH ladder below the echo — they must
                    // never merge into the previous turn's group. The
                    // pre-steer ladder is complete (it never gets its own
                    // say), so its progress animation is frozen here.
                    self.reanchor_turn_after_user_echo();
                }
                self.steer_items.retain(|(s, _)| s != seq);
            }
            SessionEvent::AutoPilot { phase, iteration } => {
                self.status = format!("autopilot: {:?} #{}", phase, iteration);
            }
            // Sidecar frames fold into the panel field `chat.sidecar` (see
            // `sidecar::fold_sidecar`): Start claims/creates the panel,
            // Child routes into the panel's nested view, Turn finalizes it.
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
        // A missing terminal display event must not leave the previous turn's
        // progress animation alive after a new prompt is admitted.
        steps::set_turn_progress(&mut self.blocks, self.turn_block_start, false);
        self.submitted = true;
        self.status.clear();
        self.turn_block_start = self.blocks.len();
        // A new admitted turn invalidates the previous run's Say anchor: a
        // repair without a fresh Say must insert, not overwrite.
        self.round_assistant_idx = None;
        self.llm_round_started_at_ms = Some(opencoder_core::message::now_ms());
        self.frozen_round_ms = None;
    }

    /// Re-anchor the live ladder floor after a user echo landed mid-flow
    /// (steer consumption, queue consumption): the echoed input opens a NEW
    /// Turn, so later steps/calls build a fresh `StepGroup` below the echo
    /// instead of merging into the previous turn's group. The previous
    /// group's progress animation is frozen — that turn ended without its
    /// own say and will never animate again. Mirrors the SPA's
    /// user-boundary rule in `steps/reducer.js` (`lastUserBoundary`).
    pub fn reanchor_turn_after_user_echo(&mut self) {
        steps::set_turn_progress(&mut self.blocks, self.turn_block_start, false);
        self.turn_block_start = self.blocks.len();
        // The echo opens a new turn boundary — same invalidation as
        // `begin_turn`: the pre-echo Say anchor must not survive as a
        // repair target for the post-echo run.
        self.round_assistant_idx = None;
    }

    /// Push a non-streamed line and ensure the next TextDelta starts a new
    /// assistant block instead of merging into a prior one.
    pub fn push_marker(&mut self, line: Line<'static>) {
        self.finalize_assistant();
        self.flush_pending_thinking();
        self.blocks
            .push(ChatBlock::Marker(vec![sanitize_line(line)]));
    }

    /// Push several non-streamed lines as a single marker block and ensure the
    /// next TextDelta starts a fresh assistant block. Used by display-only
    /// commands (e.g. `/ps`) whose multi-line echo never reaches the model.
    pub fn push_marker_lines(&mut self, lines: Vec<Line<'static>>) {
        self.finalize_assistant();
        self.flush_pending_thinking();
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

    /// Toggle the click target at flat index `call_idx` inside the StepGroup
    /// at `block_idx` (mouse click handler on a rendered ladder row). The
    /// index walks the turn's VISIBLE rows — the turn row, then (while it is
    /// open) each step row, then (while a step is open) its calls aggregation
    /// row, then (while that list is open) each function-call row, in render
    /// order — exactly what `visible_targets` and
    /// `collect_headers` enumerate, so the toggled row is the row that was
    /// clicked. A turn target flips its steps; a step target flips that
    /// step; a calls target flips the aggregate list; a call target toggles
    /// that single call's result. No-op if either index is out of range.
    pub fn toggle_tool_call_at(&mut self, block_idx: usize, call_idx: usize) {
        let Some(ChatBlock::StepGroup { open, steps, .. }) = self.blocks.get_mut(block_idx) else {
            return;
        };
        let Some(target) = steps::visible_targets(*open, steps).get(call_idx).copied() else {
            return;
        };
        match target {
            StepTarget::Group => *open = !*open,
            StepTarget::Step(si) => {
                if let Some(s) = steps.get_mut(si) {
                    s.open = !s.open;
                    if s.open {
                        steps::render_step_thinking(s);
                    }
                }
            }
            StepTarget::Calls(si) => {
                if let Some(s) = steps.get_mut(si) {
                    s.calls_open = !s.calls_open;
                }
            }
            StepTarget::Call(si, ci) => {
                if let Some(c) = steps.get_mut(si).and_then(|s| s.calls.get_mut(ci)) {
                    c.expanded = !c.expanded;
                }
            }
        }
    }

    /// Collapse every collapsible block (Thinking + Compaction + StepGroup)
    /// in this view. Bound to Ctrl+L: clears any expanded reasoning blocks
    /// and closes every level of the tool ladder — turn fold, step folds,
    /// calls-list folds and expanded call results — in one keystroke (the turn row itself
    /// always stays rendered; also applied to a child
    /// subagent view before exiting it).
    pub fn collapse_all_collapsible(&mut self) {
        for block in &mut self.blocks {
            match block {
                ChatBlock::Thinking { collapsed, .. } | ChatBlock::Compaction { collapsed, .. } => {
                    *collapsed = true;
                }
                ChatBlock::StepGroup { open, steps, .. } => {
                    *open = false;
                    for s in steps {
                        s.open = false;
                        s.calls_open = false;
                        for c in &mut s.calls {
                            c.expanded = false;
                        }
                    }
                }
                _ => {}
            }
        }
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
