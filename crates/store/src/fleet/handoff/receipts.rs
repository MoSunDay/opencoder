use super::super::FleetStore;
use anyhow::{ensure, Result};
use libsql::{params, params_from_iter, TransactionBehavior};
use opencoder_core::fleet::{Assignment, CreateExecution, ExecutionKind, RpcReply};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Project sessions retain one owner while each initial admission has its own run ID.
pub fn dispatch_key(request: &CreateExecution) -> &str {
    if request.kind == ExecutionKind::Project {
        request.input["run_id"].as_str().unwrap_or(&request.id)
    } else {
        &request.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub fingerprint: String,
    pub phase: String,
    pub payload: Value,
}

/// Dispatch-time naming snapshot lifted out of `execution_assignments` for
/// list rendering. The five-field execution index stays free of detail
/// fields (protocol locked); this is a read-only projection of the frozen
/// assignment: the placement target plus the definition name/spec name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionNames {
    pub target: Option<String>,
    pub definition_name: Option<String>,
    pub spec_name: Option<String>,
}

impl FleetStore {
    pub async fn receipt(&self, scope: &str, id: &str) -> Result<Option<Receipt>> {
        let _gate = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT fingerprint,phase,payload FROM dispatch_receipts WHERE scope=?1 AND id=?2",
                params![scope, id],
            )
            .await?;
        match rows.next().await? {
            None => Ok(None),
            Some(row) => Ok(Some(Receipt {
                fingerprint: row.get(0)?,
                phase: row.get(1)?,
                payload: serde_json::from_str(&row.get::<String>(2)?)?,
            })),
        }
    }

    /// First writer owns the original intent. Returns false on content reuse.
    pub async fn claim_request(&self, scope: &str, id: &str, fingerprint: &str) -> Result<bool> {
        let _gate = self.gate.lock().await;
        self.conn.execute(
            "INSERT INTO dispatch_receipts VALUES (?1,?2,?3,'claimed','null') ON CONFLICT(scope,id) DO NOTHING",
            params![scope,id,fingerprint],
        ).await?;
        let mut rows = self
            .conn
            .query(
                "SELECT fingerprint FROM dispatch_receipts WHERE scope=?1 AND id=?2",
                params![scope, id],
            )
            .await?;
        Ok(rows.next().await?.unwrap().get::<String>(0)? == fingerprint)
    }

    /// Caller holds request_lock for this key. Updates cannot change intent.
    pub async fn save_receipt(&self, scope: &str, id: &str, receipt: &Receipt) -> Result<()> {
        let _gate = self.gate.lock().await;
        let changed = self.conn.execute(
            "UPDATE dispatch_receipts SET phase=?4,payload=?5 WHERE scope=?1 AND id=?2 AND fingerprint=?3",
            params![scope,id,receipt.fingerprint.clone(),receipt.phase.clone(),serde_json::to_string(&receipt.payload)?],
        ).await?;
        ensure!(changed == 1, "request receipt ownership conflict");
        Ok(())
    }

    pub async fn assignment(&self, id: &str) -> Result<Option<Assignment>> {
        let _gate = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT assignment FROM execution_assignments WHERE id=?1",
                [id],
            )
            .await?;
        rows.next()
            .await?
            .map(|row| Ok(serde_json::from_str(&row.get::<String>(0)?)?))
            .transpose()
    }

    /// Batch lookup of dispatch-time names for the execution list. One SQL
    /// round trip (no N+1); ids without a stored assignment are simply absent
    /// from the map.
    pub async fn execution_names(&self, ids: &[String]) -> Result<HashMap<String, ExecutionNames>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let _gate = self.gate.lock().await;
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT id, json_extract(assignment,'$.request.target'), \
             json_extract(assignment,'$.definition.name'), \
             json_extract(assignment,'$.definition.spec.name') \
             FROM execution_assignments WHERE id IN ({placeholders})"
        );
        let args: Vec<libsql::Value> = ids
            .iter()
            .map(|id| libsql::Value::Text(id.clone()))
            .collect();
        let mut rows = self.conn.query(&sql, params_from_iter(args)).await?;
        let mut out = HashMap::with_capacity(ids.len());
        while let Some(row) = rows.next().await? {
            out.insert(
                row.get::<String>(0)?,
                ExecutionNames {
                    target: row.get::<Option<String>>(1)?,
                    definition_name: row.get::<Option<String>>(2)?,
                    spec_name: row.get::<Option<String>>(3)?,
                },
            );
        }
        Ok(out)
    }

    /// Atomic dispatch outbox: ownership, frozen input and dispatch phase exist
    /// before the first RPC. A lost reply never leads to a new assignment.
    pub async fn prepare_assignment(
        &self,
        assignment: &Assignment,
        fingerprint: &str,
    ) -> Result<()> {
        ensure!(
            assignment.index.id == assignment.request.id
                && assignment.index.kind == assignment.request.kind,
            "assignment index must match request id and kind"
        );
        let _gate = self.gate.lock().await;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let i = &assignment.index;
        let mut rows = tx
            .query(
                "SELECT assignment FROM execution_assignments WHERE id=?1",
                [i.id.as_str()],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let old: Assignment = serde_json::from_str(&row.get::<String>(0)?)?;
            ensure!(
                old.index.node_id == i.node_id
                    && old.index.kind == i.kind
                    && old.index.created_at == i.created_at,
                "assignment ownership conflict"
            );
            if old.request == assignment.request {
                return Ok(());
            }
            ensure!(
                old.request.kind == ExecutionKind::Project
                    && dispatch_key(&old.request) != dispatch_key(&assignment.request),
                "assignment conflict"
            );
            let mut rejected = tx
                .query(
                    "SELECT phase FROM dispatch_receipts WHERE scope='execution' AND id=?1",
                    [dispatch_key(&old.request)],
                )
                .await?;
            ensure!(
                rejected
                    .next()
                    .await?
                    .is_some_and(|row| row.get::<String>(0).ok().as_deref() == Some("rejected")),
                "previous project dispatch is unresolved or accepted"
            );
        }
        drop(rows);
        tx.execute("INSERT INTO execution_index(id,created_at,kind,node_id,status) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO NOTHING",
            params![i.id.clone(),i.created_at,i.kind.prefix(),i.node_id.clone(),i.status.as_str()]).await?;
        let mut rows = tx
            .query(
                "SELECT node_id,kind,created_at FROM execution_index WHERE id=?1",
                [i.id.as_str()],
            )
            .await?;
        let row = rows.next().await?.unwrap();
        ensure!(
            row.get::<String>(0)? == i.node_id
                && row.get::<String>(1)? == i.kind.prefix()
                && row.get::<i64>(2)? == i.created_at,
            "execution ownership conflict"
        );
        drop(rows);
        tx.execute(
            "INSERT INTO execution_assignments VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET assignment=excluded.assignment",
            params![i.id.clone(), serde_json::to_string(assignment)?],
        )
        .await?;
        let changed = tx.execute("UPDATE dispatch_receipts SET phase='prepared' WHERE scope='execution' AND id=?1 AND fingerprint=?2", params![dispatch_key(&assignment.request),fingerprint]).await?;
        ensure!(changed == 1, "missing dispatch receipt");
        tx.commit().await?;
        Ok(())
    }

    pub async fn finish_dispatch(
        &self,
        id: &str,
        fingerprint: &str,
        reply: &RpcReply,
    ) -> Result<()> {
        self.save_receipt(
            "execution",
            id,
            &Receipt {
                fingerprint: fingerprint.into(),
                phase: "accepted".into(),
                payload: serde_json::to_value(reply)?,
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::{ExecutionIndex, ExecutionStatus};

    async fn store() -> FleetStore {
        FleetStore::open_memory().await.unwrap()
    }

    /// Raw-JSON assignment fixture: `request_extra` lands inside
    /// `$.request`, `definition_extra` at the assignment top level.
    async fn put_raw_assignment(
        store: &FleetStore,
        id: &str,
        request_extra: &str,
        definition_extra: &str,
    ) {
        let rc = if request_extra.is_empty() { "" } else { "," };
        let dc = if definition_extra.is_empty() { "" } else { "," };
        let payload = format!(
            r#"{{"index":{{"id":"{id}","created_at":1,"kind":"dag","node_id":"n1","status":"pending"}},"request":{{"id":"{id}","kind":"dag"{rc}{request_extra}}}{dc}{definition_extra}}}"#
        );
        store
            .conn
            .execute(
                "INSERT INTO execution_assignments VALUES (?1,?2)",
                params![id, payload],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn execution_names_batches_shapes_and_skips_missing_ids() {
        let store = store().await;
        let rows: [(&str, &str, &str); 5] = [
            ("agent-a", r#""target":"coder-x""#, ""),
            (
                "dag-b",
                "",
                r#""definition":{"id":"etl","name":"etl","spec":{"name":"etl"}}"#,
            ),
            (
                "team-c",
                r#""target":"stale-name""#,
                r#""definition":{"name":"demo"}"#,
            ),
            ("dag-d", "", r#""definition":{"spec":{"name":"legacy"}}"#),
            ("brain-e", "", ""),
        ];
        for (id, request_extra, definition_extra) in rows {
            put_raw_assignment(&store, id, request_extra, definition_extra).await;
        }
        let ids: Vec<String> = rows.iter().map(|(id, ..)| (*id).to_string()).collect();
        let mut all = ids.clone();
        all.push("ghost-x".into());
        let names = store.execution_names(&all).await.unwrap();
        assert_eq!(names.len(), 5, "an id without an assignment stays absent");
        assert_eq!(names["agent-a"].target.as_deref(), Some("coder-x"));
        assert_eq!(names["agent-a"].definition_name, None);
        assert_eq!(names["dag-b"].spec_name.as_deref(), Some("etl"));
        assert_eq!(names["dag-b"].definition_name.as_deref(), Some("etl"));
        assert_eq!(names["team-c"].definition_name.as_deref(), Some("demo"));
        assert_eq!(names["team-c"].target.as_deref(), Some("stale-name"));
        assert_eq!(names["dag-d"].definition_name, None, "spec-only snapshot");
        assert_eq!(names["dag-d"].spec_name.as_deref(), Some("legacy"));
        assert_eq!(names["brain-e"], ExecutionNames::default());
    }

    #[tokio::test]
    async fn execution_names_with_empty_ids_short_circuits() {
        let store = store().await;
        put_raw_assignment(&store, "agent-a", r#""target":"coder-x""#, "").await;
        let names = store.execution_names(&[]).await.unwrap();
        assert!(
            names.is_empty(),
            "an empty id list short-circuits to an empty map"
        );
    }

    #[tokio::test]
    async fn execution_names_reads_assignments_written_by_prepare() {
        let store = store().await;
        let index = ExecutionIndex {
            id: "team-real-1".into(),
            created_at: 7,
            kind: ExecutionKind::Team,
            node_id: "n1".into(),
            status: ExecutionStatus::Pending,
        };
        let request = CreateExecution {
            id: index.id.clone(),
            kind: ExecutionKind::Team,
            target: Some("demo".into()),
            input: serde_json::json!({}),
            node_id: Some("n1".into()),
        };
        let fingerprint = opencoder_core::token_hash(&serde_json::to_string(&request).unwrap());
        assert!(
            store
                .claim_request("execution", "team-real-1", &fingerprint)
                .await
                .unwrap(),
            "test owns the dispatch receipt"
        );
        store
            .prepare_assignment(
                &Assignment {
                    private_context: None,
                    runtime: None,
                    codex: None,
                    index: index.clone(),
                    request,
                    definition: Some(serde_json::json!({"name": "demo", "captain": "act"})),
                },
                &fingerprint,
            )
            .await
            .unwrap();
        let names = store
            .execution_names(&["team-real-1".to_string(), "team-ghost".to_string()])
            .await
            .unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names["team-real-1"].target.as_deref(), Some("demo"));
        assert_eq!(
            names["team-real-1"].definition_name.as_deref(),
            Some("demo")
        );
        assert_eq!(names["team-real-1"].spec_name, None);
    }
}
