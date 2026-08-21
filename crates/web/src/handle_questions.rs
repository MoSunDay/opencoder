//! Glue between the question hub, the drain command channel, and HTTP.
//!
//! Kept out of `handle.rs` purely to respect the file-size budget: the drain
//! loop itself stays lean while these small helpers own the "web parity with
//! the TUI" semantics (abandon-on-disconnect, `/ap` + `/requirement` drain
//! commands, post-drain title generation).

use std::sync::Arc;

use opencoder_core::ApMode;
use opencoder_session::SessionState;
use opencoder_store::{SessionPatch, Store};
use tracing::warn;

use crate::handle::{HandleMap, SessionHandle};

/// Abandon every question currently waiting on this handle's hub. Called when
/// the last SSE subscriber disconnects: without it a `question` tool call
/// would block the drain turn forever with nobody left to answer. Each
/// abandoned call resolves to [`opencoder_session::tools::question::SKIPPED_REPLY`]
/// on the tool side, so the turn completes instead of hanging.
pub(crate) fn abandon_all_waiting(h: &SessionHandle) {
    for (id, _) in h.question_hub.waiting_questions() {
        h.question_hub.abandon(&id);
    }
}

/// Get-or-create the [`SessionHandle`] for `session_id` under the map lock.
/// Shared by the question/annotation/autopilot endpoints, which must reach a
/// stable handle even before the first prompt spawns a drain.
pub(crate) async fn get_or_create_handle(
    handles: &HandleMap,
    session_id: &str,
) -> Arc<SessionHandle> {
    let mut map = handles.lock().await;
    map.entry(session_id.to_string())
        .or_insert_with(SessionHandle::new)
        .clone()
}

/// Body of [`DrainCmd::SetApMode`] (TUI `/ap` parity, worker.rs ApModeSwitch):
/// set the session-scoped override AND the live config mode, then persist the
/// canonical spelling so a resume restores it. Best-effort persist — a store
/// hiccup must not kill the drain.
pub(crate) async fn apply_set_ap_mode(session: &mut SessionState, mode: ApMode) {
    session.ap_mode_override = Some(mode);
    session.config.autopilot.mode = mode;
    if let Some(store) = &session.store {
        let patch = SessionPatch {
            autopilot_mode: Some(mode.as_str().to_string()),
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        };
        if let Err(e) = store.update_session(&session.id, &patch).await {
            warn!(session_id = %session.id, error = %e, "persist autopilot mode failed");
        }
    }
}

/// Body of [`DrainCmd::SetAnnotation`] (TUI `/requirement` parity,
/// worker.rs EditAnnotation): `None`/blank is an explicit clear, otherwise
/// store the trimmed text. Best-effort persist.
pub(crate) async fn apply_set_annotation(session: &mut SessionState, text: Option<String>) {
    let effective = text.filter(|t| !t.trim().is_empty());
    let patch = match &effective {
        None => SessionPatch {
            clear_requirement: true,
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        },
        Some(t) => SessionPatch {
            requirement: Some(t.clone()),
            updated_at: Some(opencoder_core::message::now_ms()),
            ..Default::default()
        },
    };
    session.requirement = effective;
    if let Some(store) = &session.store {
        if let Err(e) = store.update_session(&session.id, &patch).await {
            warn!(session_id = %session.id, error = %e, "persist requirement failed");
        }
    }
}

/// Body of [`DrainCmd::ResetPlanPhase`]: re-entry into plan mode resets only
/// the phase input counter (the plan snapshot deliberately survives — see
/// `plan_phase::reset_plan_phase`) and persists it so a resume re-arms fresh.
pub(crate) async fn apply_reset_plan_phase(session: &mut SessionState) {
    session.reset_plan_phase();
    session.persist_plan_phase().await;
}

/// Post-drain title generation (mirrors `crates/cli/src/run.rs`): only when
/// the run completed Ok and the session row has no title yet (covers multiple
/// run attempts — the title check makes it once-only). Bounded at 30 s so a
/// hanging small-model endpoint can never wedge the drain teardown; failures
/// only log (`generate_title` already warns internally).
pub(crate) async fn maybe_generate_title(
    store: &Arc<dyn Store>,
    session: &SessionState,
    run_ok: bool,
) {
    if !run_ok {
        return;
    }
    // Mirror the CLI's guard: a cancelled session (POST /interrupt) must not
    // spend up to 30 s blocked on a possibly-hanging small-model call — the
    // drain has to exit promptly so `draining` flips false.
    if session.cancel.as_ref().is_some_and(|t| t.is_cancelled()) {
        return;
    }
    let has_title = match store.get_session(&session.id).await {
        Ok(Some(meta)) => meta.title.is_some_and(|t| !t.trim().is_empty()),
        // Unreadable/missing row: skip rather than generate a title for a
        // session that may have been deleted mid-drain.
        _ => return,
    };
    if has_title {
        return;
    }
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        opencoder_session::generate_title(session),
    )
    .await;
}
