//! Keymap-modal outcome handling extracted from `app_loop.rs` to keep that
//! file under the 800-line iteration cap. Pure move: same signatures and
//! side effects (disk save + config reload + `KeyBindings` rebuild).

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use tokio::sync::mpsc;

use super::LoopFlow;
use crate::keymap_menu::KeymapMenu;
use crate::worker::UiCmd;

/// Handle a keystroke while the keymap-rebinding modal is open. On `Save`,
/// persists the changed keymap fields to disk, reloads config, and rebuilds
/// the `KeyBindings` so the new shortcuts take effect immediately. On `Quit`,
/// sends `UiCmd::Quit` and returns [`LoopFlow::Quit`].
pub(crate) async fn handle_keymap_outcome(
    keymap_menu: &mut Option<KeymapMenu>,
    k: KeyEvent,
    config: &mut Config,
    keymap: &mut crate::keymap::KeyBindings,
    workdir: &Path,
    cmd_tx: &mpsc::Sender<UiCmd>,
) -> LoopFlow {
    use crate::keymap_menu::handle_keymap_key;
    let outcome = handle_keymap_key(keymap_menu, k);
    apply_keymap_outcome(outcome, config, keymap, workdir, cmd_tx).await
}

/// Apply a [`KeymapOutcome`](crate::keymap_menu::KeymapOutcome) (from
/// keyboard or mouse) with shared side-effects.
pub(crate) async fn apply_keymap_outcome(
    outcome: crate::keymap_menu::KeymapOutcome,
    config: &mut Config,
    keymap: &mut crate::keymap::KeyBindings,
    workdir: &Path,
    cmd_tx: &mpsc::Sender<UiCmd>,
) -> LoopFlow {
    use crate::keymap_menu::KeymapOutcome;
    match outcome {
        KeymapOutcome::Quit => {
            let _ = cmd_tx.send(UiCmd::Quit).await;
            LoopFlow::Quit
        }
        KeymapOutcome::Save(patch) => {
            if Config::save(workdir, &patch).is_ok() {
                if let Ok(new_config) = Config::load(workdir) {
                    *config = new_config;
                    *keymap = crate::keymap::KeyBindings::from_config(config);
                }
            }
            LoopFlow::Proceed
        }
        KeymapOutcome::Cancel | KeymapOutcome::Idle => LoopFlow::Proceed,
    }
}

/// Handle a mouse event while the keymap modal is open.
pub(crate) async fn handle_keymap_mouse_event(
    keymap_menu: &mut Option<KeymapMenu>,
    btn_rects: &[ratatui::layout::Rect],
    m: &crossterm::event::MouseEvent,
    config: &mut Config,
    keymap: &mut crate::keymap::KeyBindings,
    workdir: &Path,
    cmd_tx: &mpsc::Sender<UiCmd>,
) -> LoopFlow {
    let outcome = crate::keymap_menu::mouse::handle_keymap_mouse(
        keymap_menu,
        btn_rects,
        m.column,
        m.row,
        &m.kind,
    );
    apply_keymap_outcome(outcome, config, keymap, workdir, cmd_tx).await
}
