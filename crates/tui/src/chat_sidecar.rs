//! Sidecar (`/sidecar <question>`) folding and flattening — extracted from
//! `chat.rs` to keep that file within its line budget. The sidecar block
//! mirrors the subagent block's design: content streams into a nested
//! [`ChatView`] (body visible only while focused, via `compute_display`'s
//! swap) and the flattened transcript carries a single header line.
//!
//! Persistence contract (mirrors the session-side gate): sidecar frames are
//! display-only. The child's `LlmUsage` arrives **bare** (never wrapped in
//! `SidecarChild`), so the parent arm accumulates it into `tokens_total`
//! exactly like a main-task round — this module deliberately skips `LlmUsage`
//! inner frames to avoid double counting.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use opencoder_session::SessionEvent;

use super::types::SPINNER;
use crate::chat::{push_duration_span, short, ChatBlock, ChatView};
use crate::theme;

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

/// Collapse every collapsible block of the focused sidecar's nested view
/// (the Ctrl+L exit path mirrors the subagent one). No-op without focus.
pub(crate) fn collapse_focused(chat: &mut ChatView) {
    if !chat.sidecar_focus {
        return;
    }
    if let Some(ChatBlock::Sidecar { view, .. }) = chat
        .blocks
        .iter_mut()
        .rev()
        .find(|b| matches!(b, ChatBlock::Sidecar { .. }))
    {
        view.collapse_all_collapsible();
    }
}

/// Flatten the sidecar block into its single header row:
/// `⇲ sidecar <question> [● running/done/failed · Ntok · Xs] <answer summary>`
/// Mirrors the subagent header's span structure (bold label, accent kind,
/// muted prompt, status mark, duration) so copy-mode and hit-testing see the
/// same one-line shape.
#[allow(clippy::too_many_arguments)] // header rendering needs the full block state
pub(crate) fn flatten_sidecar(
    question: &str,
    done: bool,
    ok: bool,
    answer: &Option<String>,
    total_tokens: u64,
    rounds: u32,
    started_at_ms: i64,
    elapsed_ms: u64,
    anim_tick: u32,
    now_ms: i64,
) -> Vec<Line<'static>> {
    let (mark, mark_color, status_word) = if done {
        if ok {
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
    let live_elapsed = if done {
        elapsed_ms
    } else {
        (now_ms - started_at_ms).max(0) as u64
    };
    let mut spans = vec![
        Span::styled(
            "\u{2937} sidecar ",
            Style::default()
                .fg(theme::sidecar_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(question.to_string(), Style::default().fg(theme::muted())),
        Span::raw(" "),
        Span::styled(mark.to_string(), Style::default().fg(mark_color)),
        Span::raw(" "),
        Span::styled(status_word.to_string(), Style::default().fg(mark_color)),
        Span::raw(format!(" · {rounds}r · {total_tokens}tok")),
    ];
    push_duration_span(&mut spans, started_at_ms, Some(live_elapsed), now_ms);
    if done {
        if let Some(a) = answer {
            let summary = short(a, 120);
            if !summary.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    summary,
                    Style::default().fg(if ok {
                        theme::muted()
                    } else {
                        theme::err_color()
                    }),
                ));
            }
        }
    }
    vec![Line::from(spans)]
}
