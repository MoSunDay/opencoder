//! Outbound multiplexed node transport, independent of the execution adapters.
mod client;
pub mod cpu;
pub use client::run;

use opencoder_core::fleet::*;

#[async_trait::async_trait]
pub trait NodeService: Send + Sync {
    fn registration(&self) -> NodeRegistration;
    fn snapshot(&self) -> NodeSnapshot;
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>>;
    async fn handle(&self, operation: NodeOperation) -> RpcReply;
}
