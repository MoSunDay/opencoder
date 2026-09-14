//! Brain (project goals / capability library) persistence — free functions
//! over a `Connection`, mirroring the `todos.rs` layout. Writes that touch
//! more than one row run inside a single `run_tx` transaction.

use anyhow::{Context, Result};
use libsql::{params, Connection, Row};

use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainPlanRecord,
    BrainVectorHit, BrainVectorWrite,
};

/// INSERT a capability plus its exemplar inputs in one transaction. Input
/// `id` fields are ignored (autoincrement assigns fresh ids on insert);
/// `position` is taken from each record.
pub async fn create(
    conn: &Connection,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        insert_capability(conn, capability).await?;
        insert_eng_inputs(conn, eng_inputs).await
    })
    .await
}

/// UPDATE every capability field and REPLACE the exemplar inputs (delete all,
/// re-insert) in one transaction — replace semantics, so removed inputs
/// disappear atomically.
pub async fn update(
    conn: &Connection,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        update_capability_fields(conn, capability).await?;
        replace_eng_inputs(conn, &capability.id, eng_inputs).await
    })
    .await
}

/// INSERT a capability, its exemplar inputs AND its embedding in ONE
/// transaction. The caller (brain runtime) embeds beforehand and passes the
/// bytes via `vector`, so a capability can never be persisted without its
/// vector — no cross-table partial-write window.
pub async fn create_with_vector(
    conn: &Connection,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
    vector: &BrainVectorWrite,
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        insert_capability(conn, capability).await?;
        insert_eng_inputs(conn, eng_inputs).await?;
        insert_or_replace_vector(conn, &capability.id, vector).await
    })
    .await
}

/// UPDATE every capability field, REPLACE the exemplar inputs and INSERT OR
/// REPLACE the embedding in ONE transaction — replace semantics on both the
/// inputs and the vector, so a stale old vector can never keep answering
/// search after the content moved on.
pub async fn update_with_vector(
    conn: &Connection,
    capability: &BrainCapabilityRecord,
    eng_inputs: &[BrainEngInputRecord],
    vector: &BrainVectorWrite,
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        update_capability_fields(conn, capability).await?;
        replace_eng_inputs(conn, &capability.id, eng_inputs).await?;
        insert_or_replace_vector(conn, &capability.id, vector).await
    })
    .await
}

/// DELETE a capability. `brain_eng_inputs` and `brain_vectors` rows follow via
/// the schema's `ON DELETE CASCADE` (foreign_keys=ON is a connection pragma).
pub async fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM brain_capabilities WHERE id=?1", params![id])
        .await
        .context("delete brain capability")?;
    Ok(())
}

/// Fetch one capability with its exemplar inputs ordered by `position, id`.
pub async fn get(conn: &Connection, id: &str) -> Result<Option<BrainCapabilityDetail>> {
    let mut rows = conn
        .query(
            "SELECT id,capability_type,summary,input_desc,output_desc,created_at,updated_at FROM brain_capabilities WHERE id=?1",
            params![id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let capability = row_capability(&row)?;
    let eng_inputs = eng_inputs(conn, id).await?;
    Ok(Some(BrainCapabilityDetail {
        capability,
        eng_inputs,
    }))
}

/// Fetch every capability (newest first) with its exemplar inputs. One query
/// per capability for the inputs — the catalog is small, simple wins over a
/// join-aggregation.
pub async fn list(conn: &Connection) -> Result<Vec<BrainCapabilityDetail>> {
    let mut rows = conn
        .query(
            "SELECT id,capability_type,summary,input_desc,output_desc,created_at,updated_at FROM brain_capabilities ORDER BY created_at DESC, id DESC",
            (),
        )
        .await?;
    let mut ids = Vec::new();
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let capability = row_capability(&row)?;
        ids.push(capability.id.clone());
        out.push(BrainCapabilityDetail {
            capability,
            eng_inputs: Vec::new(),
        });
    }
    for (detail, id) in out.iter_mut().zip(ids) {
        detail.eng_inputs = eng_inputs(conn, &id).await?;
    }
    Ok(out)
}

/// INSERT OR REPLACE the embedding for a capability (`capability_id` is the
/// table's primary key, so a re-upsert overwrites in place — never duplicates).
/// `emb` is the little-endian f32 byte encoding consumed by `vector32`.
pub async fn upsert_vector(
    conn: &Connection,
    capability_id: &str,
    dim: i64,
    model: &str,
    emb: &[u8],
    updated_at: i64,
) -> Result<()> {
    insert_or_replace_vector(
        conn,
        capability_id,
        &BrainVectorWrite {
            dim,
            model: model.to_string(),
            emb: emb.to_vec(),
            embedded_at: updated_at,
        },
    )
    .await
}

/// Nearest-neighbour search: cosine distance between each stored embedding
/// and `query_emb`, ascending, capped at `limit`.
///
/// The query embedding is bound as a BLOB (little-endian f32 bytes) — the
/// same encoding as the stored `emb` column — because libsql's bundled
/// SQLite accepts `vector32(?)` with either a blob or a JSON-array text
/// binding (verified empirically by the integration test); blob is kept for a
/// single shared encoding. `WHERE v.model = ?2` scopes the scan to one
/// embedding model so mixed-model stores never hit a dim-mismatch error.
pub async fn search(
    conn: &Connection,
    model: &str,
    query_emb: &[u8],
    limit: u32,
) -> Result<Vec<BrainVectorHit>> {
    let mut rows = conn
        .query(
            "SELECT c.id, c.capability_type, c.summary, c.input_desc, c.output_desc, c.created_at, c.updated_at, \
             vector_distance_cos(v.emb, vector32(?1)) AS d \
             FROM brain_vectors v JOIN brain_capabilities c ON c.id = v.capability_id \
             WHERE v.model = ?2 ORDER BY d LIMIT ?3",
            params![query_emb, model, limit],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(BrainVectorHit {
            capability: row_capability(&row)?,
            distance: row.get(7)?,
        });
    }
    Ok(out)
}

/// INSERT one decision-tree plan. `id` is a fresh ULID minted by the brain
/// runtime, so a plain INSERT (no upsert) is correct — a collision would be
/// a ULID collision.
pub async fn save_plan(conn: &Connection, plan: &BrainPlanRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO brain_plans (id,situation,situation_digest,chat_model,tree_json,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            plan.id.as_str(),
            plan.situation.as_str(),
            plan.situation_digest.as_str(),
            plan.chat_model.as_str(),
            plan.tree_json.as_str(),
            plan.created_at
        ],
    )
    .await
    .context("insert brain plan")?;
    Ok(())
}

/// Fetch one plan by id (`None` if absent).
pub async fn get_plan(conn: &Connection, id: &str) -> Result<Option<BrainPlanRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,situation,situation_digest,chat_model,tree_json,created_at FROM brain_plans WHERE id=?1",
            params![id],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_plan(&row)?)),
        None => Ok(None),
    }
}

/// Newest plan for a situation digest — the dispatch-side cache probe. Ties
/// on `created_at` (same millisecond) break on `rowid` so the "latest" pick
/// is total-order stable, mirroring the node-task FIFO convention.
pub async fn latest_plan_by_digest(
    conn: &Connection,
    digest: &str,
) -> Result<Option<BrainPlanRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,situation,situation_digest,chat_model,tree_json,created_at FROM brain_plans \
             WHERE situation_digest=?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![digest],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_plan(&row)?)),
        None => Ok(None),
    }
}

/// Column order shared by every brain_plans SELECT.
fn row_plan(row: &Row) -> Result<BrainPlanRecord> {
    Ok(BrainPlanRecord {
        id: row.get(0)?,
        situation: row.get(1)?,
        situation_digest: row.get(2)?,
        chat_model: row.get(3)?,
        tree_json: row.get(4)?,
        created_at: row.get(5)?,
    })
}

async fn insert_capability(conn: &Connection, c: &BrainCapabilityRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO brain_capabilities (id,capability_type,summary,input_desc,output_desc,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            c.id.as_str(),
            c.capability_type.as_str(),
            c.summary.as_str(),
            c.input_desc.as_str(),
            c.output_desc.as_str(),
            c.created_at,
            c.updated_at
        ],
    )
    .await
    .context("insert brain capability")?;
    Ok(())
}

/// UPDATE every mutable column of one capability row — shared by `update`
/// and `update_with_vector` so the statement lives in exactly one place.
async fn update_capability_fields(conn: &Connection, c: &BrainCapabilityRecord) -> Result<()> {
    conn.execute(
        "UPDATE brain_capabilities SET capability_type=?1,summary=?2,input_desc=?3,output_desc=?4,updated_at=?5 WHERE id=?6",
        params![
            c.capability_type.as_str(),
            c.summary.as_str(),
            c.input_desc.as_str(),
            c.output_desc.as_str(),
            c.updated_at,
            c.id.as_str()
        ],
    )
    .await
    .context("update brain capability")?;
    Ok(())
}

/// INSERT OR REPLACE one brain_vectors row — shared by `upsert_vector` and
/// the combined-transaction writers so the SQL lives in exactly one place.
/// `capability_id` is the table's primary key, so a re-write overwrites in
/// place (never duplicates).
async fn insert_or_replace_vector(
    conn: &Connection,
    capability_id: &str,
    vector: &BrainVectorWrite,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO brain_vectors (capability_id,dim,model,emb,updated_at) VALUES (?1,?2,?3,?4,?5)",
        params![
            capability_id,
            vector.dim,
            vector.model.as_str(),
            vector.emb.as_slice(),
            vector.embedded_at
        ],
    )
    .await
    .context("upsert brain vector")?;
    Ok(())
}

async fn insert_eng_inputs(conn: &Connection, inputs: &[BrainEngInputRecord]) -> Result<()> {
    for input in inputs {
        conn.execute(
            "INSERT INTO brain_eng_inputs (capability_id,content,position) VALUES (?1,?2,?3)",
            params![
                input.capability_id.as_str(),
                input.content.as_str(),
                input.position
            ],
        )
        .await
        .context("insert brain eng input")?;
    }
    Ok(())
}

async fn replace_eng_inputs(
    conn: &Connection,
    capability_id: &str,
    inputs: &[BrainEngInputRecord],
) -> Result<()> {
    conn.execute(
        "DELETE FROM brain_eng_inputs WHERE capability_id=?1",
        params![capability_id],
    )
    .await
    .context("delete brain eng inputs")?;
    insert_eng_inputs(conn, inputs).await
}

async fn eng_inputs(conn: &Connection, capability_id: &str) -> Result<Vec<BrainEngInputRecord>> {
    let mut rows = conn
        .query(
            "SELECT id,capability_id,content,position FROM brain_eng_inputs WHERE capability_id=?1 ORDER BY position, id",
            params![capability_id],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(BrainEngInputRecord {
            id: Some(row.get(0)?),
            capability_id: row.get(1)?,
            content: row.get(2)?,
            position: row.get(3)?,
        });
    }
    Ok(out)
}

/// Column order shared by every brain_capabilities SELECT (incl. the vector
/// join, where the capability fields lead and the distance trails).
fn row_capability(row: &Row) -> Result<BrainCapabilityRecord> {
    Ok(BrainCapabilityRecord {
        id: row.get(0)?,
        capability_type: row.get(1)?,
        summary: row.get(2)?,
        input_desc: row.get(3)?,
        output_desc: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}
