use super::*;

impl FleetStore {
    pub async fn accept_execution(&self, accepted: &ExecutionIndex) -> Result<()> {
        let accepted = accepted.to_owned();
        self.db
            .write(move |conn| async move {
                let accepted = &accepted;
                FleetConnection { conn }.accept_execution(accepted).await
            })
            .await
    }
}
