use super::{
    cache::{Cache, Stamp},
    io, Inspection, Journal, Record, RecordLocation,
};
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::ExecutionIndex;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const ACK_KEYS: [&str; 5] = [
    "layered_wake_ack",
    "layered_guidance_ack",
    "brain_layered_ack",
    "layered_dispatch_acks",
    "layered_cancel_acks",
];

pub(super) struct ReadTicket {
    layout: crate::DirectoryLayout,
    path: PathBuf,
    index: ExecutionIndex,
    legacy: bool,
    pub revision: u64,
    cache: Arc<Mutex<Cache>>,
    inspection_cache: Arc<Mutex<Cache>>,
    slots: Arc<tokio::sync::Semaphore>,
    acks: Value,
}

impl ReadTicket {
    fn stamp(&self) -> Result<Stamp> {
        // Revalidate the entire path on every read, including cache hits.
        let path = if self.legacy {
            self.layout.legacy_record_path(&self.index.id)?
        } else {
            self.layout.record_path(self.index.kind, &self.index.id)?
        };
        ensure!(path == self.path, "journal path changed");
        Ok(Stamp::of(
            &std::fs::metadata(&path).context("stat execution record")?,
        ))
    }

    pub(super) fn read_sync(&self) -> Result<Arc<Record>> {
        let before = self.stamp()?;
        let cached = self
            .cache
            .lock()
            .unwrap()
            .get(&self.index.id, self.revision, &before);
        let bytes = match cached {
            Some(bytes) => bytes,
            None => Arc::new(std::fs::read(&self.path).context("read execution record")?),
        };
        let mut record = io::parse_record(&bytes, self.legacy)?;
        let after = Stamp::of(&std::fs::metadata(&self.path)?);
        ensure!(before == after, "execution record changed during read");
        ensure!(
            record.assignment.index == self.index,
            "execution record disagrees with journal index"
        );
        self.cache
            .lock()
            .unwrap()
            .insert(self.index.id.clone(), self.revision, after, bytes);
        // The database can durably acknowledge a report before the journal copy
        // is rewritten. Preserve that overlay when loading the original record.
        self.overlay_acks(&mut record.annotations);
        Ok(Arc::new(record))
    }

    fn overlay_acks(&self, annotations: &mut Value) {
        for key in ACK_KEYS {
            if let Some(value) = self.acks.get(key) {
                if !annotations.is_object() {
                    *annotations = serde_json::json!({});
                }
                annotations[key] = value.clone();
            }
        }
    }

    pub(super) fn inspect_sync(&self) -> Result<Inspection> {
        let before = self.stamp()?;
        let cached =
            self.inspection_cache
                .lock()
                .unwrap()
                .get(&self.index.id, self.revision, &before);
        if let Some(bytes) = cached {
            let mut inspection: Inspection = serde_json::from_slice(&bytes)?;
            ensure!(
                before == Stamp::of(&std::fs::metadata(&self.path)?),
                "execution record changed during read"
            );
            ensure!(
                inspection.index == self.index,
                "execution detail disagrees with journal index"
            );
            self.overlay_acks(&mut inspection.annotations);
            return Ok(inspection);
        }
        let record = self.read_sync()?;
        let after = Stamp::of(&std::fs::metadata(&self.path)?);
        ensure!(before == after, "execution record changed during read");
        if let Some(bytes) = Inspection::encode(&record) {
            self.inspection_cache.lock().unwrap().insert(
                self.index.id.clone(),
                self.revision,
                after,
                Arc::new(bytes),
            );
        }
        Ok(Inspection::from_record(Arc::unwrap_or_clone(record)))
    }

    async fn inspect(self) -> Result<Inspection> {
        let permit = self.slots.clone().acquire_owned().await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            self.inspect_sync()
        })
        .await?
    }

    pub(super) async fn read(self) -> Result<Arc<Record>> {
        let permit = self.slots.clone().acquire_owned().await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            self.read_sync()
        })
        .await?
    }
}

impl Journal {
    pub(super) fn ticket(&self, id: &str) -> Result<Option<ReadTicket>> {
        let Some(summary) = self.summaries.get(id) else {
            return Ok(None);
        };
        let legacy = summary.location == RecordLocation::Legacy;
        let path = if legacy {
            self.layout.legacy_record_path(id)?
        } else {
            self.layout.record_path(summary.index.kind, id)?
        };
        let mut acks = serde_json::Map::new();
        if let Some(projected) = self.records.get(id) {
            for key in ACK_KEYS {
                if let Some(value) = projected.annotations.get(key) {
                    acks.insert(key.into(), value.clone());
                }
            }
        }
        Ok(Some(ReadTicket {
            layout: self.layout.clone(),
            path,
            index: summary.index.clone(),
            legacy,
            revision: summary.revision,
            cache: self.cache.clone(),
            inspection_cache: self.inspection_cache.clone(),
            slots: self.read_slots.clone(),
            acks: Value::Object(acks),
        }))
    }

    pub(crate) async fn get(&self, id: &str) -> Result<Option<Record>> {
        match self.ticket(id)? {
            Some(ticket) => Ok(Some(Arc::unwrap_or_clone(ticket.read().await?))),
            None => Ok(None),
        }
    }

    pub(crate) fn get_sync(&self, id: &str) -> Result<Option<Record>> {
        self.ticket(id)?
            .map(|ticket| ticket.read_sync().map(Arc::unwrap_or_clone))
            .transpose()
    }
}

impl crate::Worker {
    pub(crate) async fn read_inspection(&self, id: &str) -> Result<Option<Inspection>> {
        loop {
            let Some(ticket) = self.inner.journal.lock().await.ticket(id)? else {
                return Ok(None);
            };
            let revision = ticket.revision;
            let result = ticket.inspect().await;
            let journal = self.inner.journal.lock().await;
            if journal.summaries.get(id).map(|s| s.revision) == Some(revision) {
                return result.map(Some);
            }
        }
    }

    /// Read without holding the journal mutex across disk IO. Retry if a writer
    /// committed or deleted the record before this snapshot was ready.
    pub(crate) async fn read_record(&self, id: &str) -> Result<Option<Arc<Record>>> {
        loop {
            let Some(ticket) = self.inner.journal.lock().await.ticket(id)? else {
                return Ok(None);
            };
            let revision = ticket.revision;
            let result = ticket.read().await;
            let journal = self.inner.journal.lock().await;
            if journal.summaries.get(id).map(|s| s.revision) == Some(revision) {
                return result.map(Some);
            }
        }
    }
}
