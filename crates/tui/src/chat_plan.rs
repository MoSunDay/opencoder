//! Plan-edit view-state helpers extracted from `ChatView` (in `chat.rs`) to
//! keep that file under the 800-line iteration cap. These are pure mutations
//! on the transcript blocks — no I/O — and live in a second `impl ChatView`
//! block, so all existing `chat.last_plan_text()` / `chat.update_plan_text()`
//! call sites are unchanged.

use crate::chat::{ChatBlock, ChatView};

impl ChatView {
    /// Return the editable plan text: prefer a `Plan` block's `raw` field,
    /// otherwise fall back to the last non-empty `Assistant` block's `raw`.
    /// The plan-edit flow seeds from the last assistant message when no
    /// replayed Plan card exists, so this covers both cases.
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

    /// The last non-empty assistant reply — the text the clear-context fold
    /// keeps as the continuation seed. UI-side preview mirror of the
    /// runner-side `handoff::last_assistant_text`.
    pub fn last_reply_text(&self) -> Option<String> {
        self.blocks.iter().rev().find_map(|b| match b {
            ChatBlock::Assistant { raw, .. } if !raw.trim().is_empty() => Some(raw.clone()),
            _ => None,
        })
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
