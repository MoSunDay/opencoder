pub mod app;
pub mod app_helpers;
pub mod cache_salt_menu;
pub mod chat;
pub mod chat_plan;
pub mod clipboard;
pub mod command;
pub mod composer;
pub mod fmt;
pub mod frame;
pub mod help;
pub mod image_render;
pub mod image_util;
pub mod input;
pub mod install_tools;
pub mod key_handler;
pub mod keybind;
pub mod local_cmd;
pub mod markdown;
pub mod menu;
pub mod model_menu;
pub mod model_session_switch;
pub mod plan_edit;
pub mod queue_panel;
pub mod render;
pub mod render_viewport;
pub mod resize;
pub mod selection;
pub mod session_ui;
pub mod skill_display;
pub mod skill_persist;
pub mod skill_token;
pub mod supervisor;
pub mod task;
pub mod terminal;
pub mod theme;
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
