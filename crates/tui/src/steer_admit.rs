//! Parent keyboard-steer off-loop admission (Enter while a turn runs).
//!
//! Same optimistic-temp-row + actor pattern as [`crate::queue_admitter`],
//! mirrored into `chat.steer_items` instead of the queue panel. The running
//! turn absorbs the durable row at its boundary via `claim_steers` /
//! late-steer peek, so this path is structurally incapable of firing a turn
//! interrupt (no `turn_cancel` anywhere in it — the `>` button remains the
//! interrupt route via `steer_fire::fire_steer_interrupt`). Routing through
//! the shared actor also gains idle_rekick's stranded-row restart for free.
//! Subagent steer intentionally stays inline (`subagent_input`) due to its
//! gate reserve/commit semantics.

use opencoder_store::Delivery;
use tokio::sync::mpsc;

use crate::app_helpers::{mk_input_with_images, snapshot_image_uris};
use crate::queue_admitter::{submit, AdmitReq, AdmitUiState};

/// Flash for a failed steer submit: the actor hand-off itself failed (actor
/// gone / channel saturated) and the temp row + images were rolled back —
/// the raw text stays recoverable via ↑ history because `push_history` runs
/// on every submit.
pub(crate) const STEER_SUBMIT_FAILED_FLASH: &str =
    "⚠ steer submit failed — recover text with ↑ history";

/// Optimistically submit a parent keyboard-steer: the caller passes TRIMMED
/// non-empty text (trim policy lives at the key handler); this builds the
/// `Delivery::Steer` input and delegates to the shared admitter actor. On
/// false the temp row and images were already rolled back — flash
/// [`STEER_SUBMIT_FAILED_FLASH`] and leave recovery to ↑ history.
pub(crate) fn submit_steer(
    tx: &mpsc::Sender<AdmitReq>,
    st: &mut AdmitUiState,
    steer_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    session_id: &str,
    raw: &str,
) -> bool {
    // Snapshot BEFORE submit: submit consumes pending_images into the
    // in-flight stash on the success path.
    let input = mk_input_with_images(
        session_id,
        Delivery::Steer,
        raw,
        Some(raw.to_string()),
        &snapshot_image_uris(pending_images),
    );
    submit(tx, st, steer_items, pending_images, input, raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_admitter::{apply_done, note_consumed, AdmitDone};

    fn ok_steer_done(temp_seq: i64, real_seq: i64, session: &str) -> AdmitDone {
        AdmitDone {
            temp_seq,
            result: Ok(real_seq),
            display: "d".into(),
            session_id: session.into(),
            steer: true,
        }
    }

    // (a) Submit ok: temp negative row lands in steer_items, images move
    // into the inflight stash, pending_images emptied.
    #[test]
    fn submit_ok_appends_temp_row_and_stashes_images() {
        let (tx, mut rx) = mpsc::channel::<AdmitReq>(1);
        let mut st = AdmitUiState::default();
        let mut steer_items = vec![];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(submit_steer(
            &tx,
            &mut st,
            &mut steer_items,
            &mut pending_images,
            "s",
            "stop exploring"
        ));
        assert_eq!(steer_items, vec![(-1, "stop exploring".to_string())]);
        assert!(pending_images.is_empty(), "images moved to the stash");
        assert_eq!(st.inflight.len(), 1);
        assert_eq!(st.inflight[0].images.len(), 1);
        let req = rx.try_recv().unwrap();
        assert_eq!(req.input.delivery, Delivery::Steer);
        assert_eq!(req.session_id, "s");
    }

    // (b) Closed channel: rollback — no row, images restored.
    #[test]
    fn submit_on_dead_channel_rolls_back() {
        let (tx, rx) = mpsc::channel::<AdmitReq>(1);
        drop(rx);
        let mut st = AdmitUiState::default();
        let mut steer_items = vec![];
        let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
        assert!(!submit_steer(
            &tx,
            &mut st,
            &mut steer_items,
            &mut pending_images,
            "s",
            "stop"
        ));
        assert!(steer_items.is_empty(), "temp row rolled back");
        assert_eq!(
            pending_images,
            vec![("img.png".to_string(), "alt".to_string())],
            "images restored to the composer"
        );
        assert!(st.inflight.is_empty());
    }

    // (c) Steer completion reconciles steer_items only; queue untouched.
    #[test]
    fn apply_done_steer_ok_replaces_temp_row() {
        let mut st = AdmitUiState::default();
        st.inflight.push(crate::queue_admitter::InflightAdmit {
            temp_seq: -1,
            images: vec![],
        });
        let mut steer_items = vec![(-1, "d".to_string())];
        let mut queue_items = vec![(5, "q".to_string())];
        let mut pending_images = vec![];
        let flash = apply_done(
            &mut st,
            ok_steer_done(-1, 7, "s"),
            &mut queue_items,
            &mut steer_items,
            &mut pending_images,
            "s",
        );
        assert!(flash.is_none());
        assert_eq!(steer_items, vec![(7, "d".to_string())], "temp row replaced");
        assert_eq!(queue_items, vec![(5, "q".to_string())], "queue untouched");
    }

    // (d) Steer failure: row removed, images restored, steer flash.
    #[test]
    fn apply_done_steer_err_restores_and_flashes() {
        let mut st = AdmitUiState::default();
        st.inflight.push(crate::queue_admitter::InflightAdmit {
            temp_seq: -1,
            images: vec![("img.png".to_string(), "alt".to_string())],
        });
        let mut steer_items = vec![(-1, "d".to_string())];
        let mut queue_items = vec![];
        let mut pending_images = vec![];
        let flash = apply_done(
            &mut st,
            AdmitDone {
                temp_seq: -1,
                result: Err(anyhow::anyhow!("store down")),
                display: "d".into(),
                session_id: "s".into(),
                steer: true,
            },
            &mut queue_items,
            &mut steer_items,
            &mut pending_images,
            "s",
        );
        assert_eq!(flash, Some(STEER_SUBMIT_FAILED_FLASH));
        assert!(steer_items.is_empty(), "temp row removed");
        assert_eq!(
            pending_images,
            vec![("img.png".to_string(), "alt".to_string())],
            "images restored"
        );
        assert!(st.inflight.is_empty());
    }

    // (e) Consumed race: the real seq hit the ledger before the completion
    // landed — the temp row must drop, never resurrect as a ghost.
    #[test]
    fn apply_done_steer_consumed_race_drops_temp_row() {
        let mut st = AdmitUiState::default();
        st.inflight.push(crate::queue_admitter::InflightAdmit {
            temp_seq: -1,
            images: vec![],
        });
        note_consumed(&mut st, 7);
        let mut steer_items = vec![(-1, "d".to_string())];
        let mut queue_items = vec![];
        let mut pending_images = vec![];
        let flash = apply_done(
            &mut st,
            ok_steer_done(-1, 7, "s"),
            &mut queue_items,
            &mut steer_items,
            &mut pending_images,
            "s",
        );
        assert!(flash.is_none());
        assert!(steer_items.is_empty(), "consumed row must not resurrect");
    }

    // (f) Session-switch race: the completion belongs to the OLD session —
    // drop it wholesale; no ghost row, no flash, and the stale images must
    // NOT leak back into the new session's composer.
    #[test]
    fn apply_done_session_mismatch_drops_without_restore() {
        let mut st = AdmitUiState::default();
        st.inflight.push(crate::queue_admitter::InflightAdmit {
            temp_seq: -1,
            images: vec![("img.png".to_string(), "alt".to_string())],
        });
        let mut steer_items = vec![(-1, "d".to_string())];
        let mut queue_items = vec![(3, "q".to_string())];
        let mut pending_images = vec![];
        let flash = apply_done(
            &mut st,
            ok_steer_done(-1, 7, "old"),
            &mut queue_items,
            &mut steer_items,
            &mut pending_images,
            "new",
        );
        assert!(flash.is_none(), "switch-race drop is not a failure");
        assert_eq!(steer_items, vec![(-1, "d".to_string())], "mirror untouched");
        assert_eq!(queue_items, vec![(3, "q".to_string())], "mirror untouched");
        assert!(
            pending_images.is_empty(),
            "stale images must not leak into the new session's composer"
        );
        assert!(st.inflight.is_empty(), "stash still drained");
    }

    // (g) Regression guard: a queue completion (steer=false) still
    // reconciles queue_items, not steer_items.
    #[test]
    fn apply_done_queue_done_reconciles_queue_mirror() {
        let mut st = AdmitUiState::default();
        st.inflight.push(crate::queue_admitter::InflightAdmit {
            temp_seq: -1,
            images: vec![],
        });
        let mut steer_items = vec![(9, "s".to_string())];
        let mut queue_items = vec![(-1, "d".to_string())];
        let mut pending_images = vec![];
        let flash = apply_done(
            &mut st,
            AdmitDone {
                temp_seq: -1,
                result: Ok(7),
                display: "d".into(),
                session_id: "s".into(),
                steer: false,
            },
            &mut queue_items,
            &mut steer_items,
            &mut pending_images,
            "s",
        );
        assert!(flash.is_none());
        assert_eq!(queue_items, vec![(7, "d".to_string())]);
        assert_eq!(
            steer_items,
            vec![(9, "s".to_string())],
            "steer mirror untouched"
        );
    }
}
