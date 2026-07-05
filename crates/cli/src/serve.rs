use anyhow::Result;

use crate::Cli;

pub async fn serve_run(_cli: &Cli, host: String, port: u16, web: bool) -> Result<()> {
    opencode_web::serve(host, port, web).await
}

pub async fn serve_launch(_cli: &Cli) -> Result<()> {
    opencode_web::serve("0.0.0.0".to_string(), 0, true).await
}
