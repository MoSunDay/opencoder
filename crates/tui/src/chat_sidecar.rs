//! Sidecar (`/sidecar`) folding — extracted from `chat.rs` to keep that
//! file within its line budget. The panel mirrors the subagent block's
//! design: content streams into a nested [`ChatView`] whose body is visible
//! only while focused, via `compute_display`'s swap. The panel is display
//! state on `ChatView` (field `sidecar`), NOT a `ChatBlock`: it has zero
//! `blocks` footprint by construction, so it can never perturb the
//! streaming invariants on `blocks` (delta tail-merging, tool-group tail
//! merge, finalize) and contributes ZERO lines to the flat main transcript
//! (the bypass Q/A leaves no trace there; [`purge`] clears the field).
//!
//! Persistence contract (mirrors the session-side gate): sidecar frames are
//! display-only. The child's `LlmUsage` arrives **bare** (never wrapped in
//! `SidecarChild`), so the parent arm accumulates it into `tokens_total`
//! exactly like a main-task round — this module deliberately skips `LlmUsage`
//! inner frames to avoid double counting.

use opencoder_session::SessionEvent;

use crate::chat::{short, ChatView, SidecarPanel};

/// True when the event is one of the three `Sidecar*` lifecycle frames.
fn is_sidecar_frame(sev: &SessionEvent) -> bool {
    matches!(
        sev,
        SessionEvent::SidecarStart { .. }
            | SessionEvent::SidecarChild { .. }
            | SessionEvent::SidecarTurn { .. }
    )
}

/// Fold one `Sidecar*` frame into the panel:
/// - `SidecarStart` claims the panel field (adopting the fresh placeholder)
///   and auto-focuses the sidecar box;
/// - `SidecarChild` routes the inner event into the panel's nested view
///   (bare `LlmUsage` inner frames are skipped — the parent already folded
///   them and they must not count twice);
/// - `SidecarTurn` finalizes the panel (status, answer, tokens, elapsed).
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
            // Only an OPEN panel accepts a Start: panel entry seeded
            // `chat.sidecar` and set the focus. A Start arriving with the
            // panel closed is a late frame from an already-destroyed
            // conversation (exit / `/task` switch) — swallowed.
            if !chat.sidecar_focus {
                return true;
            }
            // Adopt the fresh placeholder (empty id) in place instead of
            // replacing the panel: the panel IS the placeholder until the
            // conversation's first Start arrives.
            match chat.sidecar.as_mut() {
                Some(panel) if panel.id.is_empty() => {
                    panel.id = id.clone();
                    panel.question = short(question, 90);
                }
                _ => {
                    chat.sidecar = Some(SidecarPanel {
                        id: id.clone(),
                        question: short(question, 90),
                        view: Box::new(ChatView {
                            llm_round_started_at_ms: Some(opencoder_core::message::now_ms()),
                            ..Default::default()
                        }),
                        done: false,
                        ok: false,
                        answer: None,
                        total_tokens: 0,
                        rounds: 0,
                        started_at_ms: opencoder_core::message::now_ms(),
                        elapsed_ms: 0,
                    });
                }
            }
            chat.sidecar_focus = true;
        }
        SessionEvent::SidecarChild { id, ev } => {
            // A bare child `LlmUsage` never arrives wrapped; if one ever does,
            // drop it here so the token is not counted twice in the parent.
            if matches!(ev.as_ref(), SessionEvent::LlmUsage { .. }) {
                return true;
            }
            if let Some(panel) = chat.sidecar.as_mut().filter(|p| p.id == *id) {
                panel.view.apply(ev);
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
            if let Some(panel) = chat.sidecar.as_mut().filter(|p| p.id == *id) {
                panel.done = true;
                panel.ok = *ok;
                if !answer.trim().is_empty() {
                    panel.answer = Some(answer.clone());
                }
                // Per-conversation totals: each turn reports its own usage, the
                // panel header shows the running sum across follow-ups.
                panel.total_tokens = panel.total_tokens.saturating_add(*total_tokens);
                panel.rounds = panel.rounds.saturating_add(*rounds as u32);
                panel.elapsed_ms = *elapsed_ms;
            }
        }
        _ => {}
    }
    true
}

/// The open sidecar panel's nested view + question + conversation token
/// total, for `compute_display`'s body swap. `None` unless `sidecar_focus`
/// is set and a panel exists (one actor per session). The token total comes
/// from the `SidecarTurn` frames: the child forwards its usage BARE (never
/// wrapped), so the nested view's own `context_used` stays 0 and the Turn
/// summary is the only honest per-box context figure.
pub(crate) fn focused(chat: &ChatView) -> Option<(&ChatView, &str, u64)> {
    if !chat.sidecar_focus {
        return None;
    }
    chat.sidecar
        .as_ref()
        .map(|p| (&*p.view, p.question.as_str(), p.total_tokens))
}

/// Clear the sidecar panel and drop the focus. The exit path (ESC /
/// Ctrl+L) and the panel entry both funnel through here so the main
/// transcript never keeps a sidecar trace: the bypass Q/A is temporary, not
/// a transcript artifact. Late frames for the removed id are swallowed by
/// `fold_sidecar`'s id lookup.
pub(crate) fn purge(chat: &mut ChatView) {
    chat.sidecar = None;
    chat.sidecar_focus = false;
}
