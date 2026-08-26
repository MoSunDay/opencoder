//! Worker-side runtime for the multi-node distributed execution plan
//! (Phase 3). A node registers to a central `opencoder server`, polls for
//! dispatched tasks, executes each one locally through the real session
//! runner, and streams its events + terminal status back over REST.
//!
//! Dependency direction: this crate depends on core/llm/session/store ONLY —
//! never on `opencoder-web` — so a node binary does not pull in the HTTP
//! serving stack. The wire DTOs come from [`opencoder_core::node_protocol`]
//! (shared with the server), and the SessionEvent→wire mapping reuses the
//! session crate's canonical `sse_kind()`/`sse_data()` accessors.

pub mod batcher;
pub mod executor;
pub mod runner;
pub mod uplink;

use std::sync::Arc;

use anyhow::Result;
use opencoder_llm::ChatStream;
use tokio::sync::watch;

pub use runner::{NodeOpts, DEFAULT_CLAIM_INTERVAL, DEFAULT_HEARTBEAT_INTERVAL, REGISTER_ATTEMPTS};

/// Resolve once the watched boolean flag turns `true`. A dropped sender parks
/// forever instead of synthesizing a flip: cancellation must come from an
/// explicit `send(true)`, never from teardown races.
pub(crate) async fn await_flag(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// CLI entry point: run this machine as an execution node registered to a
/// central server. `override_client` exists for tests only (deterministic
/// [`opencoder_llm::MockChatClient`]-style backends); production passes None.
///
/// The bearer token must already be RESOLVED by the caller (the CLI applies
/// the exact client semantics: `--token` flag else `OPENCODER_SERVER_TOKEN`,
/// and never auto-generates).
pub async fn run_node(opts: NodeOpts, override_client: Option<Arc<dyn ChatStream>>) -> Result<()> {
    runner::run_node(opts, override_client).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    /// Missing token is refused at the entry gate — before any HTTP. This is
    /// the node-side analogue of the client subcommand's no-auto-generate
    /// contract, asserted here because the parse layer cannot know the env.
    #[test]
    fn missing_token_fails_fast_without_network() {
        let opts = NodeOpts {
            name: "n".into(),
            remote: "http://127.0.0.1:1".into(),
            token: String::new(),
            workdir: PathBuf::from("."),
            heartbeat_interval: Duration::from_secs(5),
            claim_interval: Duration::from_millis(1500),
            version: env!("CARGO_PKG_VERSION").into(),
            local_store_dir: None,
        };
        let err = opts.validate().unwrap_err().to_string();
        assert!(
            err.contains("OPENCODER_SERVER_TOKEN"),
            "error must name the remedy: {err}"
        );

        // Blank-but-set fields are equally invalid; a well-formed NodeOpts
        // passes without any server round-trip.
        let good = NodeOpts {
            token: "t".into(),
            ..opts
        };
        assert!(good.validate().is_ok());
    }
}
