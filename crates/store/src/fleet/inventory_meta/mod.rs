//! A bounded prefix on the existing JSON row avoids shared-schema migrations.
//! Old writers discard the optional field; the next read safely stamps it again.
use super::FleetStore;
use anyhow::{ensure, Result};
use serde_json::Value;
use turso::{
    params,
    transaction::{Transaction, TransactionBehavior},
};

const FIELD: &str = "_opencoder_inventory_revision";
const PREFIX: &str = "{\"_opencoder_inventory_revision\":\"";
const PREFIX_BYTES: i64 = 96;

#[cfg(test)]
mod tests;

pub(super) fn encode(kind: &str, body: &Value) -> Result<String> {
    if kind == "runtime_sleep" && body.is_object() {
        let encoded = if body.get(FIELD).is_some() {
            let mut body = body.clone();
            body.as_object_mut().unwrap().remove(FIELD);
            serde_json::to_string(&body)?
        } else {
            serde_json::to_string(body)?
        };
        return Ok(format!("{PREFIX}{}\",{}", ulid::Ulid::new(), &encoded[1..]));
    }
    Ok(serde_json::to_string(body)?)
}

pub(super) fn decode(kind: &str, encoded: &str) -> Result<Value> {
    let mut body: Value = serde_json::from_str(encoded)?;
    if kind == "runtime_sleep" {
        if let Some(object) = body.as_object_mut() {
            object.remove(FIELD);
        }
    }
    Ok(body)
}

fn stamp(prefix: Option<&str>) -> Option<(String, bool)> {
    match prefix {
        None | Some("null") => Some((String::new(), false)),
        Some(value) => {
            let revision = value.strip_prefix(PREFIX)?.split_once("\",")?.0;
            ulid::Ulid::from_string(revision).ok()?;
            Some((revision.to_owned(), true))
        }
    }
}

impl FleetStore {
    pub async fn runtime_sleep_stamp(&self, id: &str) -> Result<(String, bool)> {
        let key = id.to_owned();
        let prefix = self.db.read(move |conn| async move {
            let mut rows = conn.query(
                "SELECT substr(body,1,?2) FROM fleet_definitions WHERE kind='runtime_sleep' AND id=?1",
                params![key, PREFIX_BYTES],
            ).await?;
            Ok(rows.next().await?.map(|row| row.get::<String>(0)).transpose()?)
        }).await?;
        if let Some(stamp) = stamp(prefix.as_deref()) {
            return Ok(stamp);
        }
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
                let mut rows = tx
                    .query(
                        "SELECT body FROM fleet_definitions WHERE kind='runtime_sleep' AND id=?1",
                        [id.as_str()],
                    )
                    .await?;
                let body = rows
                    .next()
                    .await?
                    .map(|row| row.get::<String>(0))
                    .transpose()?;
                drop(rows);
                // Another writer may have replaced/deleted the row before this lease.
                if let Some(stamp) = stamp(body.as_deref()) {
                    tx.commit().await?;
                    return Ok(stamp);
                }
                let body: Value = serde_json::from_str(body.as_deref().unwrap())?;
                ensure!(body.is_object(), "invalid frozen runtime inventory");
                let encoded = encode("runtime_sleep", &body)?;
                let result = stamp(Some(&encoded)).expect("newly stamped inventory");
                tx.execute(
                    "UPDATE fleet_definitions SET body=?2 WHERE kind='runtime_sleep' AND id=?1",
                    params![id, encoded],
                )
                .await?;
                tx.commit().await?;
                Ok(result)
            })
            .await
    }
}
