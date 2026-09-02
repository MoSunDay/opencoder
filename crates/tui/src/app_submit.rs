//! `KeyAction::Submit` arm extracted from `app.rs`'s `run_app` event loop to
//! keep that file under the 800-line iteration cap. Pure mechanical move —
//! the logic, comments and behavior are unchanged from the original inline
//! arm; locals became parameters and `continue`/`break` became the returned
//! [`LoopFlow`] (matching the `app_loop` extraction pattern).
//!
//! `Proceed` covers both original fall-offs: the running-path `continue` and
//! the natural arm end (the `handle_key` match is the last statement of the
//! `Event::Key` arm, so both reached the next select iteration). Only
//! `start_turn` failure and a slash-command `Quit` map to [`LoopFlow::Quit`].

use std::path::Path;
use std::sync::{Arc, Mutex};

use opencoder_core::Config;
use opencoder_llm::estimate;
use opencoder_store::Store;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::app_loop::LoopFlow;
use crate::app_helpers::{push_history, push_user, snapshot_image_uris, start_turn, worker_dead};
use crate::chat::ChatView;
use crate::model_menu::ModelMenu;
use crate::queue_admitter;
use crate::skill_persist::{act_plan_highlight, resolve_persist};
use crate::worker::UiCmd;

/// Idle submit / queued submit handling for the composer's Enter action.
/// See the module docs for the control-flow translation.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_submit_action(
    text: String,
    running: &mut bool,
    admit_tx: &mpsc::Sender<queue_admitter::AdmitReq>,
    admit_st: &mut queue_admitter::AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    session_id: &str,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
    chat: &mut ChatView,
    sidecar_ask: &mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
    store: &Arc<dyn Store>,
    plan_skill_active: &mut bool,
    clear_confirm: &mut Option<crate::clear_confirm::ClearConfirm>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    notepad: &mut Option<crate::notepad::NotepadView>,
    task_picker: &mut Option<crate::task::TaskPicker>,
    model_menu: &mut Option<ModelMenu>,
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
    envs_menu: &mut Option<crate::envs_menu::EnvsMenu>,
    cli_menu: &mut Option<crate::cli_menu::CliMenu>,
    skill_toggle_menu: &mut Option<crate::skill_menu::SkillMenu>,
    ap_menu: &mut Option<crate::ap_menu::ApMenu>,
    cache_salt_menu: &mut Option<crate::cache_salt_menu::CacheSaltMenu>,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    task_elapsed_ms: &mut u64,
    cancelled: &mut bool,
    follow: &mut bool,
    body_refresh_pending: &mut bool,
) -> LoopFlow {
    if *running {
        // Submit while running is unreachable (Enter/Tab map to
        // Steer/Queue when running) — no bare slash command can
        // land here.
        // Deferred: the raw text (tokens included) queues verbatim;
        // the runner's record_compound resolves/activates/
        // persists the skill at the idle boundary — never now,
        // or it would fire inside the running turn.
        queue_admitter::handle_queue(
            &text,
            admit_tx,
            admit_st,
            queue_items,
            pending_images,
            session_id,
        );
        push_history(history, hist_idx, &text);
        return LoopFlow::Proceed;
    }
    // Idle submit: the turn starts now, so eager skill
    // activation (and persistence) is the correct timing.
    let (clean, _unresolved) = resolve_persist(
        &text,
        active_skill,
        active_skill_body,
        sys_tokens,
        agent_name,
        workdir,
        skill_handle,
        chat,
        store,
        session_id,
    )
    .await;
    *plan_skill_active = act_plan_highlight(active_skill.as_deref());
    let clean = clean.trim().to_string();
    let clean = crate::control_helpers::forward_skill_if_compound(&text, &clean);
    // Clear-context arm (both spellings, compound included):
    // countdown guard — unlike command::parse this keeps
    // the compound tail (previously leaked verbatim).
    if crate::clear_confirm::maybe_arm(
        clear_confirm,
        chat,
        mode_flash,
        anim_tick,
        &clean,
        Some(clean.clone()),
    ) {
        return LoopFlow::Proceed;
    }
    // Intercept /annotation: open the editor instead of submitting
    if let Some(action) = crate::command::parse(&clean) {
        // Unified slash-command dispatch: route recognized `/cmd`
        // through the same handler as the `/` popup picker.
        let f = super::app_loop::dispatch_slash_action(
            action,
            cmd_tx,
            cancel,
            chat,
            sidecar_ask,
            running,
            follow,
            store,
            session_id,
            task_picker,
            model_menu,
            mcp_menu,
            envs_menu,
            cli_menu,
            skill_toggle_menu,
            ap_menu,
            cache_salt_menu,
            agent_name,
            config,
            workdir,
            mode_flash,
            anim_tick,
            sys_tokens,
            plan_edit,
            notepad,
            clear_confirm,
            admit_tx,
            admit_st,
            queue_items,
            pending_images,
            history,
            hist_idx,
        )
        .await;
        match f {
            LoopFlow::Quit => return LoopFlow::Quit,
            _ => push_history(history, hist_idx, &text),
        }
    } else if clean.is_empty() {
        if active_skill.is_some() {
            if !text.is_empty() {
                push_user(chat, history, hist_idx, &text, &text);
            }
            // Skill-only submit: send a trigger prompt naming the active skill so
            // the model records a user turn and acts on the injected skill body.
            let skill_name = active_skill.as_deref().unwrap_or("");
            let trigger = crate::skill_display::skill_trigger(skill_name);
            let image_uris = snapshot_image_uris(pending_images);
            if !start_turn(cmd_tx, cancel, UiCmd::Prompt(trigger, image_uris)).await {
                worker_dead(chat);
                return LoopFlow::Quit;
            }
            pending_images.clear();
            *task_elapsed_ms = 0;
            *running = true;
            *follow = true;
            chat.begin_turn();
            *body_refresh_pending = true;
        }
    } else {
        // Echo only the model-facing text: a
        // compound control command (`/plan
        // review`) echoes just its tail — the
        // command token is applied inline and
        // never recorded. History keeps the raw
        // input for arrow-up recall.
        let echo = opencoder_session::consumed_echo_text(&clean).unwrap_or_else(|| text.clone());
        push_user(chat, history, hist_idx, &echo, &text);
        chat.context_used += estimate(&clean) as u64;
        let image_uris = snapshot_image_uris(pending_images);
        if !start_turn(cmd_tx, cancel, UiCmd::Prompt(clean, image_uris)).await {
            worker_dead(chat);
            return LoopFlow::Quit;
        }
        pending_images.clear();
        *task_elapsed_ms = 0;
        *cancelled = false;
        *running = true;
        *follow = true;
        chat.begin_turn();
        *body_refresh_pending = true;
    }
    LoopFlow::Proceed
}
