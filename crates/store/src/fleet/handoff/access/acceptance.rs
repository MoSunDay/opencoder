use super::FleetStore;
use anyhow::Result;
use opencoder_core::fleet::ExecutionIndex;

impl FleetStore {
    /// Both timely and late Create replies can confirm an execution. Node
    /// reports remain the authority for subsequent status changes.
    pub async fn accept_execution(&self, accepted: &ExecutionIndex) -> Result<()> {
        let accepted = accepted.to_owned();
        self.db
            .write(move |store| super::accept_execution(store, &accepted))
            .await
    }
}
