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
        // Keep the round's sole Assistant at the tail while inserting every
        // later Thinking segment before it. Moving the enum value is cheap and
        // preserves the accumulated raw/rendered buffers without cloning.
        let assistant = matches!(
            self.blocks.last(),
            Some(ChatBlock::Assistant { done: false, .. })
        )
        .then(|| self.blocks.pop().expect("last block checked above"));

        if let Some(ChatBlock::Thinking {
            text,
            sealed: false,
            ..
        }) = self.blocks.last_mut()
        {
            text.push_str(reasoning);
        } else {
            self.blocks.push(ChatBlock::Thinking {
                text: reasoning.to_string(),
                collapsed: true,
                sealed: false,
            });
        }

        if let Some(assistant) = assistant {
            self.blocks.push(assistant);
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
    }

    /// Replace the current turn's streamed parent answer with the reliable
    /// completed text held by `SessionState`. This is the lossless boundary
    /// for parent `TextDelta` chunks intentionally shed under UI backpressure.
    pub fn reconcile_completed_assistant(&mut self, completed: &str) {
        let completed = crate::terminal_text::sanitize_multiline(completed).into_owned();
        if completed.is_empty() {
            return;
        }

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

    /// True when the current round has a collapsed, unsealed Thinking block.
    /// During interleaving it sits immediately before the open Assistant.
    pub fn last_open_thinking_collapsed(&self) -> bool {
        self.open_thinking_index().is_some_and(|idx| {
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
