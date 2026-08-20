//! Popup overlay cluster — every `Option<&Menu>` overlay drawn above the
//! composer in one pass. Extracted verbatim from `render.rs` (plus the
//! `@file` picker) to keep that file under the 800-line cap. Geometry and
//! z-order (declaration order) are unchanged: each popup renders only when
//! its `Some`, later popups over earlier ones.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::cache_salt_menu::CacheSaltMenu;
use crate::command::CommandMenu;
use crate::file_menu::FileMenu;
use crate::keymap_menu::KeymapMenu;
use crate::model_menu::ModelMenu;
use crate::render::MouseHits;
use crate::task::TaskPicker;

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_popups(
    f: &mut Frame,
    area: Rect,
    composer_top: u16,
    hits: &mut MouseHits,
    task_picker: Option<&TaskPicker>,
    command_menu: Option<&CommandMenu>,
    file_menu: Option<&FileMenu>,
    model_menu: Option<&ModelMenu>,
    mcp_menu: Option<&crate::mcp_menu::McpMenu>,
    envs_menu: Option<&crate::envs_menu::EnvsMenu>,
    cli_menu: Option<&crate::cli_menu::CliMenu>,
    skill_toggle_menu: Option<&crate::skill_menu::SkillMenu>,
    ap_menu: Option<&crate::ap_menu::ApMenu>,
    cache_salt_menu: Option<&CacheSaltMenu>,
    keymap_menu: Option<&KeymapMenu>,
    question_menu: Option<&crate::question_menu::QuestionMenu>,
) {
    if let Some(tp) = task_picker {
        crate::task::render_task_picker(f, area, tp);
    }
    if let Some(cm) = command_menu {
        crate::command::render_command_popup(f, area, composer_top, cm);
    }
    if let Some(fm) = file_menu {
        crate::file_menu::render_file_popup(f, area, composer_top, fm);
    }
    if let Some(mm) = model_menu {
        crate::model_menu::render_model_popup(f, area, composer_top, mm);
    }
    if let Some(mcp) = mcp_menu {
        crate::mcp_menu::render_mcp_popup(f, area, composer_top, mcp);
    }
    if let Some(envs) = envs_menu {
        crate::envs_menu::render_envs_popup(f, area, composer_top, envs);
    }
    if let Some(cli) = cli_menu {
        crate::cli_menu::render_cli_popup(f, area, composer_top, cli);
    }
    if let Some(sk) = skill_toggle_menu {
        crate::skill_menu::render_skill_popup(f, area, composer_top, sk);
    }
    if let Some(am) = ap_menu {
        crate::ap_menu::render_ap_popup(f, area, composer_top, am);
    }
    if let Some(cs) = cache_salt_menu {
        crate::cache_salt_menu::render_cache_salt_popup(f, area, cs);
    }
    if let Some(km) = keymap_menu {
        crate::keymap_menu::render_keymap_popup(f, area, km, &mut hits.keymap_btns);
    }
    if let Some(qm) = question_menu {
        crate::question_menu::render_question_popup(f, area, composer_top, qm);
    }
}
