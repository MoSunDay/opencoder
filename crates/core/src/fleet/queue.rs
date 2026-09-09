use serde::{Deserialize, Serialize};

pub const MAX_NODE_RUNS: usize = 65535;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueOrder {
    #[default]
    Fifo,
    Lifo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeScheduling {
    pub max_runs: usize,
    #[serde(default)]
    pub queue_order: QueueOrder,
}

impl NodeScheduling {
    pub fn validate(self) -> Result<(), String> {
        if !(1..=MAX_NODE_RUNS).contains(&self.max_runs) {
            return Err(format!("max_runs must be between 1 and {MAX_NODE_RUNS}"));
        }
        Ok(())
    }
}

/// Stable ordering independent of client IDs and wall-clock resolution.
pub fn queue_cmp(order: QueueOrder, left: u64, right: u64) -> std::cmp::Ordering {
    match order {
        QueueOrder::Fifo => left.cmp(&right),
        QueueOrder::Lifo => right.cmp(&left),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_order_and_capacity_are_explicit() {
        assert!(queue_cmp(QueueOrder::Fifo, 1, 2).is_lt());
        assert!(queue_cmp(QueueOrder::Lifo, 1, 2).is_gt());
        for max_runs in [0, MAX_NODE_RUNS + 1] {
            assert!(NodeScheduling {
                max_runs,
                queue_order: QueueOrder::Fifo
            }
            .validate()
            .is_err());
        }
        let settings: NodeScheduling = serde_json::from_str(r#"{"max_runs":3}"#).unwrap();
        assert_eq!(settings.queue_order, QueueOrder::Fifo);
        settings.validate().unwrap();
    }
}
