//! Plan-edit view-state helpers extracted from `ChatView` (in `chat.rs`) to
//! keep that file under the 800-line iteration cap. These are pure mutations
//! on the transcript blocks — no I/O — and live in a second `impl ChatView`
//! block, so all existing `chat.last_plan_text()` / `chat.update_plan_text()`
//! call sites are unchanged.

use crate::chat::{ChatBlock, ChatView};

impl ChatView {
    /// Return the editable plan text: prefer a `Plan` block's `raw` field,
    /// otherwise fall back to the last non-empty `Assistant` block's `raw`.
    /// In plan mode the plan IS the last assistant message, so this covers
    /// both pre-handoff (no Plan block yet) and post-handoff cases.
    pub fn last_plan_text(&self) -> Option<String> {
        for block in self.blocks.iter().rev() {
            if let ChatBlock::Plan { raw, .. } = block {
                if !raw.trim().is_empty() {
                    return Some(raw.clone());
                }
            }
        }
        for block in self.blocks.iter().rev() {
            if let ChatBlock::Assistant { raw, .. } = block {
                if !raw.trim().is_empty() {
                    return Some(raw.clone());
                }
            }
        }
        None
    }

    /// Update the plan text in-place: re-render markdown on the Plan block
    /// (or the last non-empty Assistant block if no Plan block exists yet).
    pub fn update_plan_text(&mut self, text: &str) {
        let text = crate::terminal_text::sanitize_multiline(text);
        for block in self.blocks.iter_mut() {
            if let ChatBlock::Plan { raw, rendered, .. } = block {
                *raw = text.to_string();
                *rendered = crate::markdown::render(&text);
                return;
            }
        }
        for block in self.blocks.iter_mut().rev() {
            if let ChatBlock::Assistant {
                raw,
                rendered,
                done,
            } = block
            {
                if !raw.trim().is_empty() {
                    *raw = text.to_string();
                    *rendered = crate::markdown::render(&text);
                    *done = true;
                    return;
                }
            }
        }
    }
}
