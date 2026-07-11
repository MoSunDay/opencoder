pub mod app;
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
pub mod session_ui;
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
}

impl TuiOpts {
    pub fn new(model: Option<String>, agent: Option<String>, workdir: Option<PathBuf>) -> Self {
        TuiOpts {
            model,
            agent,
            workdir,
        }
    }
}

pub async fn run_tui(opts: &TuiOpts) -> Result<()> {
    app::run(opts).await
}
