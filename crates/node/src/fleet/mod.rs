//! Outbound multiplexed node transport, independent of the execution adapters.
mod client;
pub mod cpu;
pub use client::run;

use opencoder_core::fleet::*;

pub struct NodeReport {
    pub snapshot: NodeSnapshot,
    pub records: Vec<ExecutionIndex>,
    pub brain: Vec<NodeFrame>,
}

#[async_trait::async_trait]
pub trait NodeService: Send + Sync {
    async fn reconnect_allowed(&self, _remote: &str) -> anyhow::Result<bool> {
        Ok(!self.retiring())
    }
    fn retiring(&self) -> bool {
        false
    }
    fn registration(&self) -> NodeRegistration;
    fn snapshot(&self) -> NodeSnapshot;
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>>;
    async fn report(&self) -> anyhow::Result<NodeReport> {
        let records = self.indexes().await?;
        Ok(NodeReport {
            snapshot: self.snapshot(),
            records,
            brain: self.brain_frames().await?,
        })
    }
    async fn brain_frames(&self) -> anyhow::Result<Vec<NodeFrame>> {
        Ok(Vec::new())
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply;
}
