//! One bounded report cursor lets transport service heartbeats and RPCs between batches.
use super::*;

pub(super) struct ReportFrames {
    id: u64,
    records: std::vec::IntoIter<ExecutionIndex>,
    removed_ids: std::vec::IntoIter<String>,
    brain: std::vec::IntoIter<NodeFrame>,
    ended: bool,
    started: std::time::Instant,
    record_count: usize,
    brain_count: usize,
    flow_controlled: bool,
    sequence: u64,
    waiting: bool,
    last_progress: std::time::Instant,
}

pub(super) async fn begin_report<S>(
    writer: &mut S,
    report: PreparedReport,
    id: u64,
    flow_controlled: bool,
    changes: Option<delta::Changes>,
) -> Result<ReportFrames>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let mut begin = IndexReportEnvelope::begin(id).with_ack(flow_controlled);
    let removed_ids = if let Some(changes) = changes {
        begin.base_sequence = changes.base_sequence;
        if begin.base_sequence.is_some() {
            begin.mode = IndexReportMode::Delta;
        }
        changes.removed_ids
    } else {
        Vec::new()
    };
    let cursor = ReportFrames {
        id,
        record_count: report.records.len(),
        brain_count: report.brain.len(),
        records: report.records.into_iter(),
        removed_ids: removed_ids.into_iter(),
        brain: report.brain.into_iter(),
        ended: false,
        started: std::time::Instant::now(),
        flow_controlled,
        sequence: 0,
        waiting: flow_controlled,
        last_progress: std::time::Instant::now(),
    };
    // Keep the matching load snapshot adjacent to Begin. Later RPC snapshots
    // may interleave with batches, but cannot replace this report's watermark.
    send(
        writer,
        &NodeFrame::Snapshot {
            snapshot: report.snapshot,
        },
    )
    .await?;
    send(writer, &NodeFrame::IndexReport { report: begin }).await?;
    Ok(cursor)
}

impl Iterator for ReportFrames {
    type Item = NodeFrame;

    fn next(&mut self) -> Option<NodeFrame> {
        let batch: Vec<_> = self
            .records
            .by_ref()
            .take(INDEX_REPORT_BATCH_SIZE)
            .collect();
        let removed_ids: Vec<_> = self
            .removed_ids
            .by_ref()
            .take(INDEX_REPORT_BATCH_SIZE - batch.len())
            .collect();
        if !batch.is_empty() || !removed_ids.is_empty() {
            let mut report =
                IndexReportEnvelope::batch(self.id, batch).with_ack(self.flow_controlled);
            report.removed_ids = removed_ids;
            return Some(NodeFrame::IndexReport { report });
        }
        if let Some(frame) = self.brain.next() {
            return Some(frame);
        }
        if self.ended {
            return None;
        }
        self.ended = true;
        Some(NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(self.id).with_ack(self.flow_controlled),
        })
    }
}

impl ReportFrames {
    pub(super) fn check_timeout(&self) -> Result<()> {
        anyhow::ensure!(
            !self.waiting || self.last_progress.elapsed().as_secs() < 120,
            "index report acknowledgement timed out; reconnect for full synchronization"
        );
        Ok(())
    }

    pub(super) fn ready(&self) -> bool {
        !self.waiting
    }

    pub(super) fn sent(&mut self) -> Result<()> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("report acknowledgement sequence overflow")?;
        self.waiting = self.flow_controlled;
        self.last_progress = std::time::Instant::now();
        Ok(())
    }

    pub(super) fn acknowledge(&mut self, report_id: u64, sequence: u64) -> Result<()> {
        if report_id != self.id || sequence < self.sequence {
            return Ok(());
        }
        anyhow::ensure!(
            sequence == self.sequence,
            "unexpected report acknowledgement sequence"
        );
        self.waiting = false;
        self.last_progress = std::time::Instant::now();
        Ok(())
    }

    pub(super) fn log_sent(&self) {
        if self.started.elapsed().as_millis() < 250 {
            return;
        }
        tracing::info!(
            report_id = self.id,
            records = self.record_count,
            brain_frames = self.brain_count,
            elapsed_ms = self.started.elapsed().as_millis(),
            "node index report sent"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_or_missing_ack_cannot_advance_the_report_cursor() {
        let mut report = ReportFrames {
            id: 4,
            records: Vec::new().into_iter(),
            removed_ids: Vec::new().into_iter(),
            brain: Vec::new().into_iter(),
            ended: false,
            started: std::time::Instant::now(),
            record_count: 0,
            brain_count: 0,
            flow_controlled: true,
            sequence: 0,
            waiting: true,
            last_progress: std::time::Instant::now(),
        };
        report.acknowledge(3, 0).unwrap();
        assert!(!report.ready());
        report.acknowledge(4, 0).unwrap();
        assert!(report.ready());
        assert!(
            matches!(report.next(), Some(NodeFrame::IndexReport { report }) if matches!(report.part, IndexReportPart::End))
        );
        report.sent().unwrap();
        report.acknowledge(4, 0).unwrap();
        assert!(!report.ready());
        assert!(report.acknowledge(4, 2).is_err());
        report.last_progress -= std::time::Duration::from_secs(121);
        assert!(report.check_timeout().is_err());
        report.acknowledge(4, 1).unwrap();
        assert!(report.ready());
        assert!(report.next().is_none());
    }
}
