use super::*;

impl FleetStore {
    pub async fn catalog_reference_candidates(&self, name: &str) -> Result<Vec<(String, Value)>> {
        let name = name.to_owned();
        self.db
            .read(move |conn| async move {
                let name = &name;
                FleetConnection { conn }
                    .catalog_reference_candidates(name)
                    .await
            })
            .await
    }
}
