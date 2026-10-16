use super::ExecutionIndex;
use serde::{Deserialize, Serialize};

pub const INDEX_REPORT_BATCH_SIZE: usize = 256;
pub const INDEX_REPORT_ACK_HEADER: &str = "x-opencoder-index-report-ack";
pub const INDEX_REPORT_ACK_ACTION: &str = "transport.index_report_ack_v1";

/// One ordered fragment of a node's complete execution-index snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReportEnvelope {
    pub report_id: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub acknowledge: bool,
    #[serde(flatten)]
    pub part: IndexReportPart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum IndexReportPart {
    Begin,
    Batch { records: Vec<ExecutionIndex> },
    End,
}

impl IndexReportEnvelope {
    pub fn with_ack(mut self, enabled: bool) -> Self {
        self.acknowledge = enabled;
        self
    }

    pub fn begin(report_id: u64) -> Self {
        Self {
            report_id,
            acknowledge: false,
            part: IndexReportPart::Begin,
        }
    }

    pub fn batch(report_id: u64, records: Vec<ExecutionIndex>) -> Self {
        Self {
            report_id,
            acknowledge: false,
            part: IndexReportPart::Batch { records },
        }
    }

    pub fn end(report_id: u64) -> Self {
        Self {
            report_id,
            acknowledge: false,
            part: IndexReportPart::End,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_report_wire_format_stays_identical_and_new_flow_is_explicit() {
        assert_eq!(
            serde_json::to_value(IndexReportEnvelope::begin(1)).unwrap(),
            serde_json::json!({"report_id":1,"phase":"begin"})
        );
        let acknowledged = IndexReportEnvelope::begin(1).with_ack(true);
        let value = serde_json::to_value(acknowledged).unwrap();
        assert_eq!(value["acknowledge"], true);
        let legacy: IndexReportEnvelope =
            serde_json::from_value(serde_json::json!({"report_id":2,"phase":"end"})).unwrap();
        assert!(!legacy.acknowledge);
    }
}
