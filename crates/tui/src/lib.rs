pub mod app;
pub mod app_helpers;
pub mod chat;
pub mod command;
pub mod composer;
pub mod fmt;
pub mod input;
pub mod key_handler;
pub mod keybind;
pub mod markdown;
pub mod menu;
pub mod model_menu;
pub mod queue_panel;
pub mod render;
pub mod selection;
pub mod session_ui;
pub mod skill_token;
pub mod task;
pub mod terminal;
pub mod worker;

use std::path::PathBuf;

use anyhow::Result;

#[derive(Default)]
pub struct TuiOpts {
    pub model: Option<String>,
    pub agent: Option<String>,
    pub workdir: Option<PathBuf>,
    pub session: Option<String>,
}

impl TuiOpts {
    pub fn new(model: Option<String>, agent: Option<String>, workdir: Option<PathBuf>) -> Self {
        TuiOpts {
            model,
            agent,
            workdir,
            session: None,
        }
    }

    pub fn with_session(mut self, session: Option<String>) -> Self {
        self.session = session;
        self
    }
}

pub async fn run_tui(opts: &TuiOpts) -> Result<()> {
    app::run(opts).await
}
