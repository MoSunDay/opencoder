//! Sidecar (`/sidecar`) folding — extracted from `chat.rs` to keep that
//! file within its line budget. The sidecar block mirrors the subagent
//! block's design: content streams into a nested [`ChatView`] whose body is
//! visible only while focused, via `compute_display`'s swap. The block
//! contributes ZERO lines to the flat main transcript (the bypass Q/A leaves
//! no trace there; [`purge`] removes every block on exit).
//!
//! Persistence contract (mirrors the session-side gate): sidecar frames are
//! display-only. The child's `LlmUsage` arrives **bare** (never wrapped in
//! `SidecarChild`), so the parent arm accumulates it into `tokens_total`
//! exactly like a main-task round — this module deliberately skips `LlmUsage`
//! inner frames to avoid double counting.

use opencoder_session::SessionEvent;

use crate::chat::{short, ChatBlock, ChatView};

/// True when the event is one of the three `Sidecar*` lifecycle frames.
fn is_sidecar_frame(sev: &SessionEvent) -> bool {
    matches!(
        sev,
        SessionEvent::SidecarStart { .. }
            | SessionEvent::SidecarChild { .. }
            | SessionEvent::SidecarTurn { .. }
    )
}

/// Fold one `Sidecar*` frame into the transcript:
/// - `SidecarStart` pushes the block and auto-focuses the sidecar box;
/// - `SidecarChild` routes the inner event into that block's nested view
///   (bare `LlmUsage` inner frames are skipped — the parent already folded
///   them and they must not count twice);
/// - `SidecarTurn` finalizes the block (status, answer, tokens, elapsed).
///
/// Returns `true` when the frame was a sidecar frame (the caller's `apply`
/// then skips its default status-line handling). Frames for an unknown id
/// (e.g. a late frame after a `/task` switch rebuilt the view) are swallowed.
pub(crate) fn fold_sidecar(chat: &mut ChatView, sev: &SessionEvent) -> bool {
    if !is_sidecar_frame(sev) {
        return false;
    }
    match sev {
        SessionEvent::SidecarStart { id, question } => {
            // Only an OPEN panel accepts a Start: panel entry pushed the
            // empty placeholder and set the focus. A Start arriving with the
            // panel closed is a late frame from an already-destroyed
            // conversation (exit / `/task` switch) — swallowed.
            if !chat.sidecar_focus {
                return true;
            }
            // Adopt the placeholder block (empty id) in place instead of
            // pushing a second block: the panel block IS the placeholder
            // until the conversation's first Start arrives.
            let adopted = chat
                .blocks
                .iter_mut()
                .any(|b| matches!(b, ChatBlock::Sidecar { id: bid, .. } if bid.is_empty()));
            if let Some(ChatBlock::Sidecar {
                id: bid,
                question: bq,
                ..
            }) = chat
                .blocks
                .iter_mut()
                .find(|b| matches!(b, ChatBlock::Sidecar { id: bid, .. } if bid.is_empty()))
            {
                *bid = id.clone();
                *bq = short(question, 90);
            }
            if !adopted {
                chat.blocks.push(ChatBlock::Sidecar {
                    id: id.clone(),
                    question: short(question, 90),
                    view: ChatView {
                        llm_round_started_at_ms: Some(opencoder_core::message::now_ms()),
                        ..Default::default()
                    },
                    done: false,
                    ok: false,
                    answer: None,
                    total_tokens: 0,
                    rounds: 0,
                    started_at_ms: opencoder_core::message::now_ms(),
                    elapsed_ms: 0,
                });
            }
            chat.sidecar_focus = true;
        }
        SessionEvent::SidecarChild { id, ev } => {
            // A bare child `LlmUsage` never arrives wrapped; if one ever does,
            // drop it here so the token is not counted twice in the parent.
            if matches!(ev.as_ref(), SessionEvent::LlmUsage { .. }) {
                return true;
            }
            if let Some(ChatBlock::Sidecar { view, .. }) = chat
                .blocks
                .iter_mut()
                .rev()
                .find(|b| matches!(b, ChatBlock::Sidecar { id: bid, .. } if bid == id))
            {
                view.apply(ev);
            }
        }
        SessionEvent::SidecarTurn {
            id,
            ok,
            answer,
            elapsed_ms,
            total_tokens,
            rounds,
        } => {
            if let Some(ChatBlock::Sidecar {
                done,
                ok: block_ok,
                answer: block_answer,
                total_tokens: block_tokens,
                rounds: block_rounds,
                elapsed_ms: block_elapsed,
                ..
            }) = chat
                .blocks
                .iter_mut()
                .rev()
                .find(|b| matches!(b, ChatBlock::Sidecar { id: bid, .. } if bid == id))
            {
                *done = true;
                *block_ok = *ok;
                if !answer.trim().is_empty() {
                    *block_answer = Some(answer.clone());
                }
                // Per-conversation totals: each turn reports its own usage, the
                // block header shows the running sum across follow-ups.
                *block_tokens = block_tokens.saturating_add(*total_tokens);
                *block_rounds = block_rounds.saturating_add(*rounds as u32);
                *block_elapsed = *elapsed_ms;
            }
        }
        _ => {}
    }
    true
}

/// The focused sidecar block's nested view + question + conversation token
/// total, for `compute_display`'s body swap. `None` unless `sidecar_focus`
/// is set and a sidecar block exists (the last one wins — one actor per
/// session). The token total comes from the `SidecarTurn` frames: the child
/// forwards its usage BARE (never wrapped), so the nested view's own
/// `context_used` stays 0 and the Turn summary is the only honest per-box
/// context figure.
pub(crate) fn focused(chat: &ChatView) -> Option<(&ChatView, &str, u64)> {
    if !chat.sidecar_focus {
        return None;
    }
    chat.blocks.iter().rev().find_map(|b| match b {
        ChatBlock::Sidecar {
            view,
            question,
            total_tokens,
            ..
        } => Some((view, question.as_str(), *total_tokens)),
        _ => None,
    })
}

/// Destroy every sidecar block and drop the focus. The exit path (ESC /
/// Ctrl+L) and the panel entry both funnel through here so the main
/// transcript never keeps a sidecar trace: the bypass Q/A is temporary, not
/// a transcript artifact. Late frames for the removed ids are swallowed by
/// `fold_sidecar`'s id lookups.
pub(crate) fn purge(chat: &mut ChatView) {
    chat.blocks
        .retain(|b| !matches!(b, ChatBlock::Sidecar { .. }));
    chat.sidecar_focus = false;
}
