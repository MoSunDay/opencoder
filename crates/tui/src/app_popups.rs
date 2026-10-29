//! Modal-interception key chain extracted from `app.rs`'s `Event::Key` arm
//! (task picker, model/mcp/cli/skill/ap menus, question dialog, cache-salt
//! and keymap pickers, `/` command menu). Pure move: same order, same
//! side effects — `break` maps to [`PopupFlow::Quit`], every `continue` to
//! [`PopupFlow::Consumed`] (redraw-flavored `continue` to
//! [`PopupFlow::Redraw`]); [`PopupFlow::NotOpen`] means no modal was open
//! and the caller falls through to the composer key path.

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use opencoder_llm::ChatStream;
use opencoder_store::Store;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cache_salt_menu::{handle_cache_salt_key, CacheSaltMenu, CacheSaltOutcome};
use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::keymap::KeyBindings;
use crate::model_menu::ModelMenu;
use crate::render::Term;
use crate::task::{handle_task_key, TaskOutcome, TaskPick, TaskPicker};
use crate::worker::UiCmd;

/// Outcome of one popup-chain evaluation. `Redraw` additionally asks the
/// caller for a fresh frame (dirty + render_pending).
pub(crate) enum PopupFlow {
    /// No modal open: fall through to the composer key path.
    NotOpen,
    /// A modal consumed the key.
    Consumed,
    /// A modal consumed the key and wants a repaint.
    Redraw,
    /// A modal asked to quit the app.
    Quit,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_popup_key(
    k: KeyEvent,
    task_picker: &mut Option<TaskPicker>,
    selection: &mut crate::remote::Selection,
    pending_task_pick: &mut Option<TaskPick>,
    cmd_tx: &mpsc::Sender<UiCmd>,
    running: &mut bool,
    chat: &mut ChatView,
    store: &Arc<dyn Store>,
    model_menu: &mut Option<ModelMenu>,
    client: &mut Arc<dyn ChatStream>,
    config: &mut Config,
    model_label: &mut String,
    compaction_threshold: &mut u64,
    context_limit: &mut u64,
    frame_ms: &mut u64,
    frame_ticker: &mut tokio::time::Interval,
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
    cli_menu: &mut Option<crate::cli_menu::CliMenu>,
    skill_toggle_menu: &mut Option<crate::skill_menu::SkillMenu>,
    ap_menu: &mut Option<crate::ap_menu::ApMenu>,
    question_menu: &mut Option<crate::question_menu::QuestionMenu>,
    question_hub: &Arc<opencoder_session::QuestionHub>,
    terminal: &mut Term,
    cache_salt_menu: &mut Option<CacheSaltMenu>,
    keymap: &mut KeyBindings,
    keymap_menu: &mut Option<crate::keymap_menu::KeymapMenu>,
    command_menu: &mut Option<CommandMenu>,
    sidecar_ask: &mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
    cancel: &mut CancellationToken,
    follow: &mut bool,
    session_id: &str,
    agent_name: &str,
    input: &mut String,
    cursor_idx: &mut usize,
    workdir: &Path,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    sys_tokens: &mut u64,
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    notepad: &mut Option<crate::notepad::NotepadView>,
    clear_confirm: &mut Option<crate::clear_confirm::ClearConfirm>,
    agent_menu: &mut Option<crate::agent_menu::AgentMenu>,
    subtask_view: &mut Option<crate::subtask_detail::SubtaskView>,
) -> PopupFlow {
    // Task picker modal: intercept all keys while open.
    if task_picker.is_some() {
        match handle_task_key(task_picker, k) {
            TaskOutcome::Pick(pick) => {
                *selection = None;
                *pending_task_pick = Some(pick);
            }
            TaskOutcome::Quit => {
                let _ = cmd_tx.send(UiCmd::Quit).await;
                return PopupFlow::Quit;
            }
            TaskOutcome::ClearAll { keep_session_id } => {
                super::app_task::handle_clear_all(
                    keep_session_id,
                    *running,
                    task_picker,
                    chat,
                    store,
                )
                .await;
            }
            TaskOutcome::Idle => {}
        }
        return PopupFlow::Consumed;
    }
    if model_menu.is_some() {
        let flow = super::app_loop::handle_model_outcome(
            model_menu,
            k,
            client,
            config,
            model_label,
            compaction_threshold,
            context_limit,
            frame_ms,
            frame_ticker,
            cmd_tx,
            chat,
            workdir,
        )
        .await;
        return match flow {
            super::app_loop::LoopFlow::Quit => PopupFlow::Quit,
            super::app_loop::LoopFlow::Redraw => PopupFlow::Redraw,
            _ => PopupFlow::Consumed,
        };
    }
    if mcp_menu.is_some() {
        let _ =
            super::app_loop::handle_mcp_outcome(mcp_menu, k, config, cmd_tx, chat, workdir).await;
        return PopupFlow::Consumed;
    }
    if cli_menu.is_some() {
        let _ =
            super::app_loop::handle_cli_outcome(cli_menu, k, config, cmd_tx, chat, workdir).await;
        return PopupFlow::Consumed;
    }
    // `/skill` toggle modal (after cli/mcp blocks, which return `Consumed` first).
    if skill_toggle_menu.is_some() {
        let _ = super::app_loop::handle_skill_outcome(
            skill_toggle_menu,
            k,
            config,
            cmd_tx,
            chat,
            workdir,
        )
        .await;
        return PopupFlow::Consumed;
    }
    // `/ap` mode-picker modal (same slot.take() pattern as /skill).
    if ap_menu.is_some() {
        let _ = super::app_loop::handle_ap_outcome(ap_menu, k, config, cmd_tx, chat, workdir).await;
        return PopupFlow::Consumed;
    }
    // Question dialog: answers resolve on the hub, mid-turn.
    if question_menu.is_some() {
        // Wrap width mirrors the popup renderer so Up/Down cursor movement
        // tracks the drawn wrapped rows.
        let q_width = terminal
            .size()
            .map(|r| crate::question_menu::input_wrap_width(r.width))
            .unwrap_or(55);
        crate::question_menu::route_question_key(question_menu, k, question_hub, q_width);
        return PopupFlow::Redraw;
    }
    if cache_salt_menu.is_some() {
        if matches!(
            handle_cache_salt_key(cache_salt_menu, k),
            CacheSaltOutcome::Quit
        ) {
            let _ = cmd_tx.send(UiCmd::Quit).await;
            return PopupFlow::Quit;
        }
        return PopupFlow::Consumed;
    }
    if keymap_menu.is_some() {
        if let super::app_loop::LoopFlow::Quit =
            super::app_loop::handle_keymap_outcome(keymap_menu, k, config, keymap, workdir, cmd_tx)
                .await
        {
            return PopupFlow::Quit;
        }
        return PopupFlow::Redraw;
    }
    // `/` command picker: intercept all keys while open.
    if command_menu.is_some() {
        let flow = super::app_loop::dispatch_command(
            command_menu,
            k,
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
            keymap_menu,
            agent_name,
            input,
            cursor_idx,
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
        return match flow {
            super::app_loop::LoopFlow::Quit => PopupFlow::Quit,
            super::app_loop::LoopFlow::Redraw => PopupFlow::Redraw,
            _ => PopupFlow::Consumed,
        };
    }
    PopupFlow::NotOpen
}
