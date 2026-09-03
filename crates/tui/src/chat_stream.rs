//! One-round streaming reducer for interleaved reasoning and answer deltas.
//!
//! Providers may emit `reasoning -> text -> reasoning -> text` in one LLM
//! round. The transcript invariant is multiple Thinking blocks followed by at
//! most one Assistant block; answer fragments are never split merely because
//! reasoning resumed between them.

use opencoder_llm::estimate;

use super::{ChatBlock, ChatView};

impl ChatView {
    pub(super) fn append_text_delta(&mut self, text: &str) {
        // The progress hint belongs to the pre-Say phase. Remove it on the
        // first real Say chunk, not at ToolEnd, so the inter-round gap stays
        // visibly alive without overlapping the answer itself.
        if !text.is_empty() {
            super::steps::set_turn_progress(&mut self.blocks, self.turn_block_start, false);
        }
        self.seal_open_thinking();
        if let Some(ChatBlock::Assistant {
            raw, done: false, ..
        }) = self.blocks.last_mut()
        {
            raw.push_str(text);
            return;
        }
        self.blocks.push(ChatBlock::Assistant {
            raw: text.to_string(),
            rendered: Vec::new(),
            done: false,
        });
    }

    pub(super) fn append_reasoning_delta(&mut self, reasoning: &str) {
        // Thinking is a structural part of a step: the delta streams
        // straight into the ladder (see `steps::append_step_thinking_delta`).
        // The admitted turn boundary, not block adjacency, owns the ladder:
        // interleaved Say blocks stay in place while reasoning updates the
        // turn's one StepGroup.
        if let Some(at) = super::steps::append_step_thinking_delta(
            &mut self.blocks,
            self.turn_block_start,
            reasoning,
        ) {
            if let Some(h) = self.hidden_assistant_idx {
                if h >= at {
                    self.hidden_assistant_idx = Some(h + 1);
                }
            }
        }
    }

    /// Fold any pending Thinking run into the step ladder (see
    /// `steps::flush_pending_thinking`) and keep `hidden_assistant_idx`
    /// pointing at the same block when the flush inserts a group at or
    /// before it. No-op without pending thinking.
    pub(crate) fn flush_pending_thinking(&mut self) {
        if let Some(at) = super::steps::flush_pending_thinking(&mut self.blocks) {
            if let Some(h) = self.hidden_assistant_idx {
                if h >= at {
                    self.hidden_assistant_idx = Some(h + 1);
                }
            }
        }
    }

    /// Finalize the current round's trailing Thinking/Assistant group.
    /// Idempotence comes from the per-block `sealed` and `done` flags.
    pub fn finalize_assistant(&mut self) {
        self.seal_open_thinking();
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
        // Round end for the ladder: account the streamed step thinking once
        // (per-step `sealed` flag). Disclosure state is user-owned: sealing
        // must not close a turn or step the user opened while it streamed.
        self.seal_trailing_step();
    }

    /// Seal the trailing step's streamed thinking into `context_used`
    /// (idempotent via `Step::sealed`) without changing any disclosure state.
    /// An interleaved round's finalized Say rides on top of its group, so the
    /// walk skips trailing sealed Assistants.
    fn seal_trailing_step(&mut self) {
        let floor = self.turn_block_start.min(self.blocks.len());
        let Some(idx) = self.blocks[floor..]
            .iter()
            .position(|block| matches!(block, ChatBlock::StepGroup { .. }))
            .map(|relative| floor + relative)
        else {
            return;
        };
        let ChatBlock::StepGroup { steps, .. } = &mut self.blocks[idx] else {
            unreachable!("step-group index was matched above");
        };
        // Only an UNSEALED trailing step belongs to the round being
        // finalized. Sealed groups are past rounds' and must not be touched.
        if let Some(step) = steps.last_mut() {
            if !step.sealed {
                self.context_used += estimate(&step.thinking_raw) as u64;
                // Finalization is the one eager render for a hidden step;
                // streaming deltas themselves remain O(1) source appends.
                super::steps::render_step_thinking(step);
                step.sealed = true;
            }
        }
    }

    /// Replace the current turn's streamed parent answer with the reliable
    /// completed text held by `SessionState`. This is the lossless boundary
    /// for parent `TextDelta` chunks intentionally shed under UI backpressure.
    pub fn reconcile_completed_assistant(&mut self, completed: &str) {
        let completed = crate::terminal_text::sanitize_multiline(completed).into_owned();
        if completed.is_empty() {
            return;
        }

        super::steps::set_turn_progress(&mut self.blocks, self.turn_block_start, false);

        let floor = self.turn_block_start.min(self.blocks.len());
        let assistant_idx = self.blocks[floor..]
            .iter()
            .rposition(|block| matches!(block, ChatBlock::Assistant { .. }))
            .map(|relative| floor + relative);

        self.seal_thinking_range(floor, assistant_idx.unwrap_or(self.blocks.len()));
        if let Some(idx) = assistant_idx {
            if let ChatBlock::Assistant {
                raw,
                rendered,
                done,
            } = &mut self.blocks[idx]
            {
                if *done {
                    self.context_used = self.context_used.saturating_sub(estimate(raw) as u64);
                }
                *raw = completed;
                *rendered = crate::markdown::render(raw);
                *done = true;
                self.context_used += estimate(raw) as u64;
            }
            return;
        }

        // `Done` may already have appended its empty separator. Insert the
        // recovered Say block before trailing empty markers so block order
        // remains Thinking -> Say -> separator even when every text delta was
        // shed before reaching the UI.
        let insert_at = self.blocks[floor..]
            .iter()
            .rposition(|block| !is_empty_marker(block))
            .map_or(floor, |relative| floor + relative + 1);
        let rendered = crate::markdown::render(&completed);
        self.context_used += estimate(&completed) as u64;
        self.blocks.insert(
            insert_at,
            ChatBlock::Assistant {
                raw: completed,
                rendered,
                done: true,
            },
        );
    }

    /// True when the current round's unsealed step reasoning is hidden by a
    /// collapsed turn or step. Used to coalesce invisible delta-only batches.
    pub fn last_open_thinking_collapsed(&self) -> bool {
        let floor = self.turn_block_start.min(self.blocks.len());
        let step_hidden = self.blocks[floor..].iter().find_map(|block| match block {
            ChatBlock::StepGroup { steps, open, .. } => steps
                .last()
                .filter(|step| !step.sealed)
                .map(|step| !*open || !step.open),
            _ => None,
        });
        step_hidden == Some(true)
            || self.open_thinking_index().is_some_and(|idx| {
                matches!(
                    self.blocks.get(idx),
                    Some(ChatBlock::Thinking {
                        collapsed: true,
                        sealed: false,
                        ..
                    })
                )
            })
    }

    fn seal_open_thinking(&mut self) {
        let Some(idx) = self.open_thinking_index() else {
            return;
        };
        if let ChatBlock::Thinking { text, sealed, .. } = &mut self.blocks[idx] {
            if !*sealed {
                self.context_used += estimate(text) as u64;
                *sealed = true;
            }
        }
    }

    fn seal_thinking_range(&mut self, start: usize, end: usize) {
        for block in &mut self.blocks[start..end] {
            if let ChatBlock::Thinking { text, sealed, .. } = block {
                if !*sealed {
                    self.context_used += estimate(text) as u64;
                    *sealed = true;
                }
            }
        }
    }

    fn open_thinking_index(&self) -> Option<usize> {
        let last_idx = self.blocks.len().checked_sub(1)?;
        match self.blocks.get(last_idx) {
            Some(ChatBlock::Thinking { sealed: false, .. }) => Some(last_idx),
            Some(ChatBlock::Assistant { done: false, .. }) => {
                let thinking_idx = last_idx.checked_sub(1)?;
                matches!(
                    self.blocks.get(thinking_idx),
                    Some(ChatBlock::Thinking { sealed: false, .. })
                )
                .then_some(thinking_idx)
            }
            _ => None,
        }
    }
}

fn is_empty_marker(block: &ChatBlock) -> bool {
    matches!(
        block,
        ChatBlock::Marker(lines)
            if lines.iter().all(|line| line.spans.iter().all(|span| span.content.is_empty()))
    )
}
