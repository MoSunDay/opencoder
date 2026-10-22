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

pub struct SourceReport {
    pub report: NodeReport,
    pub revision: u64,
    pub full: bool,
    pub removed_ids: Vec<String>,
}

#[async_trait::async_trait]
// async_trait annotates futures that are already must-use on Rust 1.99.
#[allow(clippy::double_must_use)]
pub trait NodeService: Send + Sync {
    async fn reconnect_allowed(&self, _remote: &str) -> anyhow::Result<bool> {
        Ok(!self.retiring())
    }
    fn retiring(&self) -> bool {
        false
    }
    /// Only managed Hosts have commit fences on every compatible older Server.
    /// Direct Workers retain full reports until their legacy writers are fenced.
    fn supports_index_deltas(&self) -> bool {
        false
    }
    fn registration(&self) -> NodeRegistration;
    fn snapshot(&self) -> NodeSnapshot;
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>>;
    /// An optional producer-side change cursor; it advances only after commit.
    async fn index_changes(&self, _after: Option<u64>) -> anyhow::Result<Option<SourceReport>> {
        Ok(None)
    }
    async fn report(&self) -> anyhow::Result<NodeReport> {
        let mut report = self.index_report().await?;
        report.brain = self.brain_frames().await?;
        Ok(report)
    }
    async fn index_report(&self) -> anyhow::Result<NodeReport> {
        let records = self.indexes().await?;
        Ok(NodeReport {
            snapshot: self.snapshot(),
            records,
            brain: Vec::new(),
        })
    }
    async fn brain_frames(&self) -> anyhow::Result<Vec<NodeFrame>> {
        Ok(Vec::new())
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply;
}
