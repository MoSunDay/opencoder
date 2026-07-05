pub mod app;
pub mod chat;
pub mod composer;
pub mod fmt;
pub mod keybind;
pub mod menu;

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
        TuiOpts { model, agent, workdir }
    }
}

pub async fn run_tui(opts: &TuiOpts) -> Result<()> {
    app::run(opts).await
}
