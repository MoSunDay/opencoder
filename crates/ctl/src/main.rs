//! `opencoder-cli` binary entry: parse, resolve the connection context,
//! dispatch, exit. Exit codes: 0 ok, 1 transport/setup, 2 auth, 4 rejection.

use clap::Parser;
use opencoder_cli::{out, Cli};

fn main() {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let code = runtime.block_on(async move {
        match opencoder_cli::run(cli).await {
            Ok(code) => code,
            Err(error) => {
                out::fail_transport(&format!("{error:#}"));
                1
            }
        }
    });
    std::process::exit(code);
}
