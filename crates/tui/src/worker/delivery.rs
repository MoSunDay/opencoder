//! Lossless ordered delivery from the synchronous runner to the bounded UI.
//!
//! AssistantFinal only covers the last answer of a run. Dropping a delta
//! from an interim or interrupted answer can permanently lose Markdown
//! delimiters even when every Say is finalized. Coalesce adjacent text
//! chunks under pressure; never discard text or cross an event boundary.

use super::{SessionEvent, UiEvent};
use tokio::sync::mpsc;

const MAX_BATCH_EVENTS: usize = 64;
const MAX_BATCH_BYTES: usize = 32 * 1024;

pub(super) fn forward_event(tx: &mpsc::UnboundedSender<UiEvent>, event: SessionEvent) {
    let _ = tx.send(UiEvent::Session(event));
}

pub(super) fn spawn_ui_event_forwarder(
    tx: mpsc::Sender<UiEvent>,
) -> (mpsc::UnboundedSender<UiEvent>, tokio::task::JoinHandle<()>) {
    let (pending_tx, mut pending_rx) = mpsc::unbounded_channel::<UiEvent>();
    let handle = tokio::spawn(async move {
        let mut next = None;
        while let Some(mut event) = match next.take() {
            Some(event) => Some(event),
            None => pending_rx.recv().await,
        } {
            // Wait before collecting a batch so a slow UI's backlog can
            // become one append. An uncongested stream incurs no delay.
            let Ok(permit) = tx.reserve().await else {
                break;
            };
            if let UiEvent::Session(SessionEvent::TextDelta(text)) = &mut event {
                for _ in 1..MAX_BATCH_EVENTS {
                    if text.len() >= MAX_BATCH_BYTES {
                        break;
                    }
                    match pending_rx.try_recv() {
                        Ok(UiEvent::Session(SessionEvent::TextDelta(chunk))) => {
                            text.push_str(&chunk);
                        }
                        Ok(boundary) => {
                            next = Some(boundary);
                            break;
                        }
                        Err(_) => break,
                    }
                }
            }
            permit.send(event);
        }
    });
    (pending_tx, handle)
}
