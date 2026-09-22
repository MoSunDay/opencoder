// Method group 2; assembled before async_trait expands the complete contract.
macro_rules! store_contract_1 {
    ($($methods:tt)*) => {
        store_contract_2! {
            $($methods)*
    async fn todo_events_page(
        &self,
        _workflow_id: &str,
        _after_seq: i64,
        _limit: u32,
        _payload_budget: usize,
    ) -> Result<TodoEventPage> {
        anyhow::bail!("bounded todo event pagination is unsupported by this store")
    }
    async fn todo_events_before(
        &self,
        _workflow_id: &str,
        _before_seq: i64,
        _limit: u32,
        _payload_budget: usize,
    ) -> Result<TodoEventPage> {
        anyhow::bail!("reverse todo event pagination is unsupported by this store")
    }
    async fn last_todo_event_seq(&self, _workflow_id: &str) -> Result<i64> {
        anyhow::bail!("todo event watermark is unsupported by this store")
    }
    async fn todo_event_payload_chunk(
        &self,
        _workflow_id: &str,
        _seq: i64,
        _offset: u64,
        _max_bytes: usize,
    ) -> Result<Option<crate::PayloadChunkRecord>> {
        anyhow::bail!("bounded todo event payload reads are unsupported by this store")
    }
    /// Persist a new brain capability together with its exemplar inputs in
    /// one transaction (project goals / capability library). Step-wise write:
    /// no embedding row is touched, so runtime paths must prefer
    /// [`create_brain_capability_with_vector`], which commits capability +
    /// eng_inputs + vector atomically; this variant stays for direct
    /// store-level tooling/tests.
    async fn create_brain_capability(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Update every capability field and replace its exemplar inputs
    /// atomically. Step-wise write: the embedding row is left untouched, so
    /// runtime paths must prefer [`update_brain_capability_with_vector`],
    /// which swaps content + eng_inputs + vector in one transaction (no
    /// stale-vector window); this variant stays for direct store-level
    /// tooling/tests.
    async fn update_brain_capability(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Delete a capability; exemplar inputs and vectors cascade.
    async fn delete_brain_capability(&self, _id: &str) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Fetch one capability with its ordered exemplar inputs.
    async fn get_brain_capability(&self, _id: &str) -> Result<Option<BrainCapabilityDetail>> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Fetch every capability (newest first) with its exemplar inputs.
    async fn list_brain_capabilities(&self) -> Result<Vec<BrainCapabilityDetail>> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Insert-or-replace the embedding for a capability. `emb` is the
    /// little-endian f32 byte encoding shared with vector search.
    async fn upsert_brain_vector(
        &self,
        _capability_id: &str,
        _dim: i64,
        _model: &str,
        _emb: &[u8],
        _updated_at: i64,
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Create a capability, replace its exemplar inputs and upsert its
    /// embedding in ONE transaction. The vector is embedded by the caller
    /// (brain runtime) beforehand and passed in as raw bytes, eliminating the
    /// cross-table partial-write window where a capability row is persisted
    /// but its vector is missing.
    async fn create_brain_capability_with_vector(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
        _vector: &BrainVectorWrite,
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Update every capability field, replace its exemplar inputs and
    /// INSERT OR REPLACE its embedding in ONE transaction. The vector is
    /// embedded by the caller (brain runtime) beforehand and passed in as
    /// raw bytes, eliminating the cross-table partial-write window where the
    /// capability content is new but a stale old vector still answers search.
    async fn update_brain_capability_with_vector(
        &self,
        _capability: &BrainCapabilityRecord,
        _eng_inputs: &[BrainEngInputRecord],
        _vector: &BrainVectorWrite,
    ) -> Result<()> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Nearest-neighbour cosine-distance search over stored embeddings,
    /// scoped to one embedding model, ascending by distance.
    async fn search_brain_vectors(
        &self,
        _model: &str,
        _query_emb: &[u8],
        _limit: u32,
    ) -> Result<Vec<BrainVectorHit>> {
        anyhow::bail!(
            "brain capabilities are not supported by {}",
            self.backend_name()
        )
    }
    /// Register (or re-register) a worker node by its unique `name`. A new
    /// name gets a fresh ULID; a known name keeps its `id` so dispatched tasks
    /// keep their foreign key, while version/workdir/last_seen_at are
    /// refreshed and `last_status` resets to `online`.
    async fn register_node(
        &self,
        _name: &str,
        _version: Option<&str>,
        _workdir: Option<&str>,
        _addr: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::NodeRecord> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    async fn list_nodes(&self) -> Result<Vec<crate::types::NodeRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    async fn get_node(&self, _id: &str) -> Result<Option<crate::types::NodeRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Delete a worker node; cascades to its node_tasks and from there to each
    /// task's synthetic session (`ON DELETE CASCADE` chain in the schema).
    async fn delete_node(&self, _id: &str) -> Result<()> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Platform-user directory (`platform_users`, v24). Tokens are stored as
    /// sha256 hex digests only; the control API returns plaintext exactly
    /// once at creation. Backends without the table keep these defaults.
    async fn find_user_by_token_hash(
        &self,
        _token_hash: &str,
    ) -> Result<Option<crate::users::PlatformUser>> {
        Ok(None)
    }
    async fn find_user_by_name(&self, _name: &str) -> Result<Option<crate::users::PlatformUser>> {
        Ok(None)
    }
    async fn list_users(&self) -> Result<Vec<crate::users::PlatformUser>> {
        Ok(Vec::new())
    }
    async fn create_user(
        &self,
        _name: &str,
        _token_hash: &str,
        _role: opencoder_core::identity::Role,
        _created_at: i64,
    ) -> Result<crate::users::PlatformUser> {
        anyhow::bail!("user store API is not supported by {}", self.backend_name())
    }
    /// Delete by name; `Ok(false)` when the user does not exist.
    async fn delete_user(&self, _name: &str) -> Result<bool> {
        anyhow::bail!("user store API is not supported by {}", self.backend_name())
    }
    /// Atomic delete that never removes the last admin: the admin-count
    /// guard runs inside the same statement as the delete, closing the
    /// count-then-delete TOCTOU two concurrent deletions could race through.
    async fn delete_user_guarding_last_admin(
        &self,
        _name: &str,
    ) -> Result<crate::users::GuardedDelete> {
        anyhow::bail!("user store API is not supported by {}", self.backend_name())
    }
    /// Re-point an existing user's credential at a new token digest
    /// (seed-token rotation); `Ok(false)` when no row carries that name.
    async fn update_user_token_hash(&self, _name: &str, _token_hash: &str) -> Result<bool> {
        anyhow::bail!("user store API is not supported by {}", self.backend_name())
    }
    async fn count_admin_users(&self) -> Result<i64> {
        Ok(0)
    }
    /// Liveness touch + cancel-command poll in one transaction: refreshes
    /// `last_seen_at`, collapses non-busy status to `idle`, and returns the
    /// ids of this node's cancelling tasks as the cancel instructions.
    async fn heartbeat_node(&self, _id: &str, _now_ms: i64) -> Result<Vec<String>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Enqueue a node task plus its synthetic session (`task_type == "node"`)
    /// atomically. The task starts `pending` and stays queued until the node
    /// claims it via [`Store::claim_next_node_task`].
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_node_task(
        &self,
        _task_id: &str,
        _session_id: &str,
        _node_id: &str,
        _title: Option<&str>,
        _prompt: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::NodeTaskRecord> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Enqueue a node task bound to an EXISTING session (the console's
    /// "continue this dialog" flow): only the `node_tasks` row is created, the
    /// session row is reused as-is. Errors when the session does not exist so
    /// the HTTP layer can answer 400 instead of dangling the FK.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_node_task_for_session(
        &self,
        _task_id: &str,
        _session_id: &str,
        _node_id: &str,
        _title: Option<&str>,
        _prompt: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
        _now_ms: i64,
    ) -> Result<crate::types::NodeTaskRecord> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Atomically claim the oldest pending task of `node_id` (FIFO, CAS-guarded
    /// so concurrent claimers never double-dispatch). Returns `None` when the
    /// node already runs a task (single-active-task policy) or nothing is due.
    async fn claim_next_node_task(
        &self,
        _node_id: &str,
        _now_ms: i64,
    ) -> Result<Option<crate::types::NodeTaskRecord>> {
        anyhow::bail!("node store API is not supported by {}", self.backend_name())
    }
    /// Move a node task along its state machine (`pending/running/cancelling`
    /// toward `done|error|cancelled`). Illegal transitions error out; terminal
    /// writes stamp `finished_at` and release the node's busy slot.

        }
    };
}
