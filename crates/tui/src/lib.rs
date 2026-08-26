pub mod ap_menu;
pub mod app;
pub mod app_helpers;
pub mod attach_badge;
pub mod bash_exec;
pub mod cache_salt_menu;
pub mod chat;
pub mod chat_plan;
pub mod chat_req;
pub mod cli_menu;
pub mod clipboard;
pub mod command;
pub mod composer;
pub mod control_helpers;
pub mod copy_mode;
pub mod copy_wrap;
pub mod envs_menu;
pub mod file_menu;
pub mod fmt;
pub mod frame;
pub mod idle_rekick;
pub mod image_chunk;
pub mod image_render;
pub mod image_util;
pub mod input;
pub mod key_handler;
pub mod keymap;
pub mod keymap_menu;
pub mod local_cmd;
pub mod markdown;
pub mod mcp_menu;
pub mod menu;
pub mod model_menu;
pub mod model_session_switch;
pub mod notepad;
pub mod onboarding;
pub mod plan_edit;
pub mod question_menu;
pub mod queue_admitter;
pub mod queue_panel;
pub mod render;
pub mod render_viewport;
pub mod resize;
pub mod scope_dialog;
pub mod scrollbar;
pub mod session_ui;
pub mod skill_display;
pub mod skill_menu;
pub mod skill_persist;
pub mod skill_token;
pub mod supervisor;
pub mod task;
pub mod terminal;
pub mod terminal_text;
pub mod theme;
pub mod tmux_bar;
pub mod tmux_mouse;
pub mod ts_mirror;
pub mod undo;
pub mod vim;
pub mod welcome;
pub mod worker;

use std::path::PathBuf;

use anyhow::Result;

#[derive(Default)]
pub struct TuiOpts {
    pub workdir: Option<PathBuf>,
    pub session: Option<String>,
    pub model: Option<String>,
}

impl TuiOpts {
    pub fn new(workdir: Option<PathBuf>) -> Self {
        TuiOpts {
            workdir,
            session: None,
            model: None,
        }
    }

    pub fn with_session(mut self, session: Option<String>) -> Self {
        self.session = session;
        self
    }

    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }
}

pub async fn run_tui(opts: &TuiOpts) -> Result<()> {
    app::run(opts).await
}
