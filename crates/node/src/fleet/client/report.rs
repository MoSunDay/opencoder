//! One bounded report cursor lets transport service heartbeats and RPCs between batches.
use super::*;

pub(super) struct ReportFrames {
    id: u64,
    records: std::vec::IntoIter<ExecutionIndex>,
    brain: std::vec::IntoIter<NodeFrame>,
    ended: bool,
    started: std::time::Instant,
    record_count: usize,
    brain_count: usize,
    flow_controlled: bool,
    sequence: u64,
    waiting: bool,
}

pub(super) async fn begin_report<S>(
    writer: &mut S,
    report: PreparedReport,
    id: u64,
    flow_controlled: bool,
) -> Result<ReportFrames>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let cursor = ReportFrames {
        id,
        record_count: report.records.len(),
        brain_count: report.brain.len(),
        records: report.records.into_iter(),
        brain: report.brain.into_iter(),
        ended: false,
        started: std::time::Instant::now(),
        flow_controlled,
        sequence: 0,
        waiting: flow_controlled,
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
    send(
        writer,
        &NodeFrame::IndexReport {
            report: IndexReportEnvelope::begin(id).with_ack(flow_controlled),
        },
    )
    .await?;
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
        if !batch.is_empty() {
            return Some(NodeFrame::IndexReport {
                report: IndexReportEnvelope::batch(self.id, batch).with_ack(self.flow_controlled),
            });
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
    pub(super) fn ready(&self) -> bool {
        !self.waiting
    }

    pub(super) fn sent(&mut self) -> Result<()> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("report acknowledgement sequence overflow")?;
        self.waiting = self.flow_controlled;
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
