// Method group: execution ledger (brain capability library + node registry)
// and identity (platform users + access tokens). Thin adapters like
// part_1: clone args, run the domain free-function under the engine's write
// lease (mutations) or the lease-free read path (queries).
use opencoder_core::identity::Role;

use crate::rdb_store::{execution, identity};
use crate::types::NodeRecord;
use crate::users::{AccessToken, GuardedDelete, PlatformUser};
use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainVectorHit,
    BrainVectorWrite,
};

macro_rules! rdb_impl_1 {
    ($($methods:tt)*) => {
        rdb_impl_2! {
            $($methods)*

    async fn create_brain_capability(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        let (capability, eng_inputs) = (capability.clone(), eng_inputs.to_vec());
        self.db
            .write(move |db| execution::brain::create(db, &capability, &eng_inputs))
            .await
    }
    async fn create_brain_capability_with_vector(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
    ) -> Result<()> {
        let (capability, eng_inputs, vector) =
            (capability.clone(), eng_inputs.to_vec(), vector.clone());
        self.db
            .write(move |db| {
                execution::brain::create_with_vector(db, &capability, &eng_inputs, &vector)
            })
            .await
    }
    async fn update_brain_capability(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        let (capability, eng_inputs) = (capability.clone(), eng_inputs.to_vec());
        self.db
            .write(move |db| execution::brain::update(db, &capability, &eng_inputs))
            .await
    }
    async fn update_brain_capability_with_vector(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
    ) -> Result<()> {
        let (capability, eng_inputs, vector) =
            (capability.clone(), eng_inputs.to_vec(), vector.clone());
        self.db
            .write(move |db| {
                execution::brain::update_if_version(db, &capability, &eng_inputs, &vector, None)
            })
            .await
    }
    async fn update_brain_capability_if_version(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
        expected: Option<&str>,
    ) -> Result<()> {
        let (capability, eng_inputs, vector, expected) = (
            capability.clone(),
            eng_inputs.to_vec(),
            vector.clone(),
            expected.map(str::to_owned),
        );
        self.db
            .write(move |db| {
                execution::brain::update_if_version(
                    db,
                    &capability,
                    &eng_inputs,
                    &vector,
                    expected.as_deref(),
                )
            })
            .await
    }
    async fn delete_brain_capability(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.write(move |db| execution::brain::delete(db, &id)).await
    }
    async fn get_brain_capability(&self, id: &str) -> Result<Option<BrainCapabilityDetail>> {
        let id = id.to_string();
        self.db.read(move |db| execution::brain::get(db, &id)).await
    }
    async fn list_brain_capabilities(&self) -> Result<Vec<BrainCapabilityDetail>> {
        self.db.read(move |db| execution::brain::list(db)).await
    }
    async fn upsert_brain_vector(
        &self,
        capability_id: &str,
        dim: i64,
        model: &str,
        emb: &[u8],
        updated_at: i64,
    ) -> Result<()> {
        let (capability_id, model, emb) =
            (capability_id.to_string(), model.to_string(), emb.to_vec());
        self.db
            .write(move |db| {
                execution::brain::upsert_vector(db, &capability_id, dim, &model, &emb, updated_at)
            })
            .await
    }
    async fn search_brain_vectors(
        &self,
        model: &str,
        query_emb: &[u8],
        limit: u32,
    ) -> Result<Vec<BrainVectorHit>> {
        let (model, query_emb) = (model.to_string(), query_emb.to_vec());
        self.db
            .read(move |db| execution::brain::search(db, &model, &query_emb, limit))
            .await
    }
    async fn register_node(
        &self,
        name: &str,
        version: Option<&str>,
        workdir: Option<&str>,
        addr: Option<&str>,
        now_ms: i64,
    ) -> Result<NodeRecord> {
        let (name, version, workdir, addr) = (
            name.to_string(),
            version.map(str::to_owned),
            workdir.map(str::to_owned),
            addr.map(str::to_owned),
        );
        self.db
            .write(move |db| {
                execution::nodes::register(
                    db,
                    &name,
                    version.as_deref(),
                    workdir.as_deref(),
                    addr.as_deref(),
                    now_ms,
                )
            })
            .await
    }
    async fn list_nodes(&self) -> Result<Vec<NodeRecord>> {
        self.db.read(move |db| execution::nodes::list(db)).await
    }
    async fn get_node(&self, id: &str) -> Result<Option<NodeRecord>> {
        let id = id.to_string();
        self.db.read(move |db| execution::nodes::get(db, &id)).await
    }
    async fn delete_node(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.write(move |db| execution::nodes::delete(db, &id)).await
    }
    async fn heartbeat_node(&self, id: &str, now_ms: i64) -> Result<Vec<String>> {
        let id = id.to_string();
        self.db
            .write(move |db| execution::nodes::heartbeat(db, &id, now_ms))
            .await
    }
    async fn find_user_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PlatformUser>> {
        let token_hash = token_hash.to_string();
        self.db
            .read(move |db| identity::find_by_token_hash(db, &token_hash))
            .await
    }
    async fn find_user_by_name(&self, name: &str) -> Result<Option<PlatformUser>> {
        let name = name.to_string();
        self.db.read(move |db| identity::find_by_name(db, &name)).await
    }
    async fn list_users(&self) -> Result<Vec<PlatformUser>> {
        self.db.read(move |db| identity::list_users(db)).await
    }
    async fn create_user(
        &self,
        name: &str,
        token_hash: &str,
        role: Role,
        created_at: i64,
    ) -> Result<PlatformUser> {
        let (name, token_hash) = (name.to_string(), token_hash.to_string());
        self.db
            .write(move |db| identity::create_user(db, &name, &token_hash, role, created_at))
            .await
    }
    async fn delete_user(&self, name: &str) -> Result<bool> {
        let name = name.to_string();
        self.db.write(move |db| identity::delete_user(db, &name)).await
    }
    async fn delete_user_guarding_last_admin(
        &self,
        name: &str,
    ) -> Result<GuardedDelete> {
        let name = name.to_string();
        self.db
            .write(move |db| identity::delete_guarding_last_admin(db, &name))
            .await
    }
    async fn update_user_token_hash(&self, name: &str, token_hash: &str) -> Result<bool> {
        let (name, token_hash) = (name.to_string(), token_hash.to_string());
        self.db
            .write(move |db| identity::update_token_hash(db, &name, &token_hash))
            .await
    }
    async fn update_user_role(&self, name: &str, role: Role) -> Result<bool> {
        let name = name.to_string();
        self.db.write(move |db| identity::update_role(db, &name, role)).await
    }
    async fn list_access_tokens(&self) -> Result<Vec<AccessToken>> {
        self.db.read(move |db| identity::tokens::list(db)).await
    }
    async fn create_access_token(&self, token: &AccessToken, hash: &str) -> Result<()> {
        let (token, hash) = (token.clone(), hash.to_string());
        self.db
            .write(move |db| identity::tokens::create(db, &token, &hash))
            .await
    }
    async fn revoke_access_token(&self, id: &str, now: i64) -> Result<bool> {
        let id = id.to_string();
        self.db
            .write(move |db| identity::tokens::revoke(db, &id, now))
            .await
    }
    async fn count_admin_users(&self) -> Result<i64> {
        self.db.read(move |db| identity::count_admins(db)).await
    }
        }
    };
}
