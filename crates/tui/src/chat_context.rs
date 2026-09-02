//! Context-window accounting for `ChatView`: the provider-truth
//! `real_context_tokens` plus the local `context_used` estimate walk.

use opencoder_llm::estimate;
use opencoder_session::SessionEvent;

use super::ChatView;

impl ChatView {
    /// Accumulate estimated token counts for this view's OWN transcript only.
    /// Child subagent tokens are excluded — each child ChatView tracks its own
    /// subtree via its own `apply` (events route through `SubagentChild`).
    pub(super) fn track_context(&mut self, ev: &SessionEvent) {
        // Note: TextDelta/ReasoningDelta are intentionally NOT counted here.
        // Counting per-delta made the status bar's ctx% indicator jump on
        // every token.
        // Instead they are counted once at round boundaries via
        // `finalize_assistant` (and `append_text_delta` for the
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
            // Queue-consumed and steer-consumed prompts are real user messages
            // the model sees in context. Previously they were echoed as
            // ChatBlock::User but silently absent from context_used, causing
            // the ctx meter to under-report by the full token size of every
            // queued/steered prompt — the main source of "displayed 70k but
            // compaction triggered at 128k" confusion.
            SessionEvent::QueueConsumed { text, .. } => {
                self.context_used += estimate(text) as u64;
            }
            SessionEvent::SteerConsumed { text, .. } => {
                self.context_used += estimate(text) as u64;
            }
            _ => {}
        }
    }
}
