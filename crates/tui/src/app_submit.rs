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
use opencoder_llm::{estimate, ChatStream};
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
    agent_menu: &mut Option<crate::agent_menu::AgentMenu>,
    // The `@capability` mention path (dispatch slot + catalog) and the
    // `/subtasks` fullscreen view slot (trailing params, mirroring the
    // agent_menu precedent).
    subtask_ui: &mut crate::subtask_events::SubtaskUi,
    client_llm: &Arc<dyn ChatStream>,
    subtask_view: &mut Option<crate::subtask_detail::SubtaskView>,
) -> LoopFlow {
    if *running {
        // Submit while running is unreachable (Enter/Tab map to
        // Steer/Queue when running) — no bare slash command can
        // land here.
        // Deferred: the raw text (tokens included) queues verbatim;
        // the runner's record_compound resolves/activates/
        // persists the skill at the idle boundary — never now,
        // or it would fire inside the running turn.
        // Channel gone/saturated → temp row and images already rolled back;
        // text recoverable via ↑ history (push_history below still runs).
        crate::app_helpers::queue_submit_flash(
            &text,
            admit_tx,
            admit_st,
            queue_items,
            pending_images,
            session_id,
            anim_tick,
            mode_flash,
        );
        push_history(history, hist_idx, &text);
        return LoopFlow::Proceed;
    }
    // `@<capability> <question>`: one-shot subtask dispatch to a
    // registered Server Agent/Operator. The child runs server-side; the
    // result is appended back with Ctrl+Enter or from the /subtasks view.
    // Unknown @-tokens fall through to the normal submit (@file mentions
    // are unaffected).
    if mention_dispatch(
        &text,
        chat,
        subtask_ui,
        store,
        config,
        client_llm,
        workdir,
        session_id,
        agent_name,
        history,
        hist_idx,
        mode_flash,
        anim_tick,
    ) {
        return LoopFlow::Proceed;
    }
    // Idle submit: the turn starts now, so eager skill
    // activation (and persistence) is the correct timing.
    let clean = if chat.remote {
        text.clone()
    } else {
        resolve_persist(
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
        .await
        .0
    };
    *plan_skill_active = act_plan_highlight(active_skill.as_deref());
    let clean = clean.trim().to_string();
    let clean = crate::control_helpers::forward_skill_if_compound(&text, &clean);
    // Clear-context arm (both spellings, compound included):
    // countdown guard — unlike command::parse this keeps
    // the compound tail (previously leaked verbatim).
    if !chat.remote
        && crate::clear_confirm::maybe_arm(
            clear_confirm,
            chat,
            mode_flash,
            anim_tick,
            &clean,
            Some(clean.clone()),
        )
    {
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
            agent_menu,
            subtask_view,
        )
        .await;
        match f {
            LoopFlow::Quit => return LoopFlow::Quit,
            _ => push_history(history, hist_idx, &text),
        }
    } else if clean.is_empty() {
        if active_skill.is_some() && !text.is_empty() {
            push_user(chat, history, hist_idx, &text, &text);
            // Skill-only submit: the RAW `$name` goes to the runner, which
            // resolves it at the consumption boundary and records a synthetic
            // `SKILL_TRIGGER` carrying the verbatim token as `Message.display`
            // — echo/replay surfaces show the user's input, never a resolved
            // trigger body. (The runner-side activation is idempotent against
            // resolve_persist's: same body lands on the shared handle.)
            let image_uris = snapshot_image_uris(pending_images);
            if !start_turn(cmd_tx, cancel, UiCmd::Prompt(text.clone(), image_uris)).await {
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
        // Echo the VERBATIM input: the compound tail keeps its `$skill`
        // tokens (`/plan $review` echoes `$review`) — recorded `blocks` stay
        // clean for the LLM, the echo is display-only. History keeps the
        // raw input for arrow-up recall.
        let echo = opencoder_session::consumed_echo_text(&text).unwrap_or_else(|| text.clone());
        if chat.remote {
            // Server consumption events own the user echo.
            // Render it once when the input reaches the remote runner.
            push_history(history, hist_idx, &text);
        } else {
            push_user(chat, history, hist_idx, &echo, &text);
            chat.context_used += estimate(&clean) as u64;
        }
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


/// `@<capability> <question>` one-shot subtask dispatch: route the
/// submit to a registered Server Agent/Operator instead of a local turn. The
/// child execution runs server-side (its `tui_remote_*` bookmark session and
/// the parent registry entry are created by `remote::subtasks::dispatch`);
/// the main-loop dispatch poller flashes completion and spawns the watcher.
/// Returns `true` when the input was consumed as a subtask dispatch.
#[allow(clippy::too_many_arguments)]
fn mention_dispatch(
    text: &str,
    chat: &ChatView,
    subtask_ui: &mut crate::subtask_events::SubtaskUi,
    store: &Arc<dyn Store>,
    config: &Config,
    client_llm: &Arc<dyn ChatStream>,
    workdir: &Path,
    session_id: &str,
    agent_name: &str,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
) -> bool {
    // Remote sessions cannot own a local parent registry entry.
    if chat.remote {
        return false;
    }
    let Some((capability_id, question)) =
        crate::remote::subtasks::parse_mention(text, &subtask_ui.caps)
    else {
        return false;
    };
    let Some(capability) = subtask_ui
        .caps
        .iter()
        .find(|capability| capability.id == capability_id)
        .cloned()
    else {
        return false;
    };
    subtask_ui.dispatch = Some(crate::remote::subtasks::request_dispatch(
        Arc::clone(store),
        config.clone(),
        Arc::clone(client_llm),
        workdir.to_path_buf(),
        capability,
        question,
        session_id.to_string(),
        agent_name.to_string(),
    ));
    *mode_flash = Some((format!("正在提交 subtask 给 {capability_id}…"), anim_tick));
    // The composer already cleared the draft (the Submit key path); keep the
    // raw mention for arrow-up recall, mirroring the normal submit history.
    if history.last().map(String::as_str) != Some(text) {
        history.push(text.to_string());
    }
    *hist_idx = None;
    true
}
