macro_rules! store_implementation_2 {
    ($($methods:tt)*) => {
        store_implementation_3! {
            $($methods)*
    async fn todo_events_before(
        &self,
        workflow_id: &str,
        before_seq: i64,
        limit: u32,
        payload_budget: usize,
    ) -> Result<TodoEventPage> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        todos::events_before(&conn, workflow_id, before_seq, limit, payload_budget).await

        }).await
    }
    async fn last_todo_event_seq(&self, workflow_id: &str) -> Result<i64> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        let mut rows = conn
            .query(
                "SELECT COALESCE(MAX(seq),0) FROM todo_events WHERE workflow_id=?1",
                [workflow_id],
            )
            .await?;
        Ok(rows
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing event watermark"))?
            .get(0)?)

        }).await
    }
    async fn todo_event_payload_chunk(
        &self,
        workflow_id: &str,
        seq: i64,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        let workflow_id = workflow_id.to_owned();
        self.db.read(move |conn| async move {
            let workflow_id = workflow_id.as_str();

        todos::event_payload_chunk(&conn, workflow_id, seq, offset, max_bytes).await

        }).await
    }
    async fn create_brain_capability(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        let capability = capability.to_owned();
        let eng_inputs = eng_inputs.to_owned();
        self.db.write(move |conn| async move {
            let capability = &capability;
            let eng_inputs = &eng_inputs;

        brain::create(&conn, capability, eng_inputs).await

        }).await
    }
    async fn update_brain_capability(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        let capability = capability.to_owned();
        let eng_inputs = eng_inputs.to_owned();
        self.db.write(move |conn| async move {
            let capability = &capability;
            let eng_inputs = &eng_inputs;

        brain::update(&conn, capability, eng_inputs).await

        }).await
    }
    async fn delete_brain_capability(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        brain::delete(&conn, id).await

        }).await
    }
    async fn get_brain_capability(&self, id: &str) -> Result<Option<BrainCapabilityDetail>> {
        let id = id.to_owned();
        self.db.read(move |conn| async move {
            let id = id.as_str();

        brain::get(&conn, id).await

        }).await
    }
    async fn list_brain_capabilities(&self) -> Result<Vec<BrainCapabilityDetail>> {
        self.db.read(move |conn| async move {

        brain::list(&conn).await

        }).await
    }
    async fn upsert_brain_vector(
        &self,
        capability_id: &str,
        dim: i64,
        model: &str,
        emb: &[u8],
        updated_at: i64,
    ) -> Result<()> {
        let capability_id = capability_id.to_owned();
        let model = model.to_owned();
        let emb = emb.to_owned();
        self.db.write(move |conn| async move {
            let capability_id = capability_id.as_str();
            let model = model.as_str();
            let emb = &emb;

        brain::upsert_vector(&conn, capability_id, dim, model, emb, updated_at).await

        }).await
    }
    async fn create_brain_capability_with_vector(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
    ) -> Result<()> {
        let capability = capability.to_owned();
        let eng_inputs = eng_inputs.to_owned();
        let vector = vector.to_owned();
        self.db.write(move |conn| async move {
            let capability = &capability;
            let eng_inputs = &eng_inputs;
            let vector = &vector;

        brain::create_with_vector(&conn, capability, eng_inputs, vector).await

        }).await
    }
    async fn update_brain_capability_with_vector(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
    ) -> Result<()> {
        let capability = capability.to_owned();
        let eng_inputs = eng_inputs.to_owned();
        let vector = vector.to_owned();
        self.db.write(move |conn| async move {
            let capability = &capability;
            let eng_inputs = &eng_inputs;
            let vector = &vector;

        brain::update_with_vector(&conn, capability, eng_inputs, vector).await

        }).await
    }
    async fn update_brain_capability_if_version(
        &self,
        capability: &BrainCapabilityRecord,
        eng_inputs: &[BrainEngInputRecord],
        vector: &BrainVectorWrite,
        expected: Option<&str>,
    ) -> Result<()> {
        let capability = capability.to_owned();
        let eng_inputs = eng_inputs.to_owned();
        let vector = vector.to_owned();
        let expected = expected.map(ToOwned::to_owned);
        self.db.write(move |conn| async move {
            let capability = &capability;
            let eng_inputs = &eng_inputs;
            let vector = &vector;
            let expected = expected.as_deref();

        brain::update_with_vector_if_version(&conn, capability, eng_inputs, vector, expected).await

        }).await
    }
    async fn search_brain_vectors(
        &self,
        model: &str,
        query_emb: &[u8],
        limit: u32,
    ) -> Result<Vec<BrainVectorHit>> {
        let model = model.to_owned();
        let query_emb = query_emb.to_owned();
        self.db.read(move |conn| async move {
            let model = model.as_str();
            let query_emb = &query_emb;

        brain::search(&conn, model, query_emb, limit).await

        }).await
    }
    async fn find_user_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<crate::users::PlatformUser>> {
        let token_hash = token_hash.to_owned();
        self.db.read(move |conn| async move {
            let token_hash = token_hash.as_str();

        users::find_by_token_hash(&conn, token_hash).await

        }).await
    }
    async fn find_user_by_name(&self, name: &str) -> Result<Option<crate::users::PlatformUser>> {
        let name = name.to_owned();
        self.db.read(move |conn| async move {
            let name = name.as_str();

        users::find_by_name(&conn, name).await

        }).await
    }
    async fn list_users(&self) -> Result<Vec<crate::users::PlatformUser>> {
        self.db.read(move |conn| async move {

        users::list(&conn).await

        }).await
    }
    async fn create_user(
        &self,
        name: &str,
        token_hash: &str,
        role: opencoder_core::identity::Role,
        created_at: i64,
    ) -> Result<crate::users::PlatformUser> {
        let name = name.to_owned();
        let token_hash = token_hash.to_owned();
        self.db.write(move |conn| async move {
            let name = name.as_str();
            let token_hash = token_hash.as_str();

        users::create(&conn, name, token_hash, role, created_at).await

        }).await
    }
    async fn delete_user(&self, name: &str) -> Result<bool> {
        let name = name.to_owned();
        self.db.write(move |conn| async move {
            let name = name.as_str();

        users::delete(&conn, name).await

        }).await
    }
    async fn delete_user_guarding_last_admin(
        &self,
        name: &str,
    ) -> Result<crate::users::GuardedDelete> {
        let name = name.to_owned();
        self.db.write(move |conn| async move {
            let name = name.as_str();

        users::delete_guarding_last_admin(&conn, name).await

        }).await
    }
    async fn update_user_token_hash(&self, name: &str, token_hash: &str) -> Result<bool> {
        let name = name.to_owned();
        let token_hash = token_hash.to_owned();
        self.db.write(move |conn| async move {
            let name = name.as_str();
            let token_hash = token_hash.as_str();

        users::update_token_hash(&conn, name, token_hash).await

        }).await
    }
    async fn update_user_role(&self, name: &str, role: opencoder_core::identity::Role) -> Result<bool> {
        let name = name.to_owned();
        self.db.write(move |conn| async move {
            let name = name.as_str();

        users::update_role(&conn,name,role).await

        }).await
    }
    async fn list_access_tokens(&self) -> Result<Vec<crate::AccessToken>> {
        self.db.read(move |conn| async move {

        users::tokens::list(&conn).await

        }).await
    }
    async fn create_access_token(&self, token: &crate::AccessToken, hash: &str) -> Result<()> {
        let token = token.to_owned();
        let hash = hash.to_owned();
        self.db.write(move |conn| async move {
            let token = &token;
            let hash = hash.as_str();

        users::tokens::create(&conn,token,hash).await

        }).await
    }
    async fn revoke_access_token(&self, id: &str, now: i64) -> Result<bool> {
        let id = id.to_owned();
        self.db.write(move |conn| async move {
            let id = id.as_str();

        users::tokens::revoke(&conn,id,now).await

        }).await
    }
    async fn count_admin_users(&self) -> Result<i64> {
        self.db.read(move |conn| async move {

        users::count_admins(&conn).await

        }).await
    }
        }
    };
}
