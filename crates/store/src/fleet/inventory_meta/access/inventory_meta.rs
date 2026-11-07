use super::FleetStore;
use anyhow::Result;

impl FleetStore {
    pub async fn runtime_sleep_stamp(&self, id: &str) -> Result<(String, bool)> {
        let key = id.to_owned();
        if let Some(stamp) = self
            .db
            .read(move |store| super::stamped(store, &key))
            .await?
        {
            return Ok(stamp);
        }
        let key = id.to_owned();
        self.db
            .write(move |store| super::runtime_sleep_stamp(store, &key))
            .await
    }
}
