//! Keep a retiring listener available to requests already accepted by ingress.

use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIdentity {
    pub pid: u32,
    pub start_ticks: u64,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retirement {
    #[serde(default)]
    pub ingress_workers: Vec<WorkerIdentity>,
    pub successor_port: Option<u16>,
}

impl Retirement {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.successor_port == Some(0)
            || (!self.ingress_workers.is_empty() && self.successor_port.is_none())
        {
            return Err("guarded retirement requires a positive successor port");
        }
        if self
            .ingress_workers
            .iter()
            .any(|worker| worker.pid == 0 || worker.start_ticks == 0)
        {
            return Err("ingress worker identity must include a positive PID and start time");
        }
        Ok(())
    }
}

fn matches_process(stat: &str, expected: u64) -> io::Result<bool> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid ingress process stat");
    let (_, fields) = stat.rsplit_once(") ").ok_or_else(invalid)?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let start_ticks: u64 = fields
        .get(19)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    Ok(fields.first() != Some(&"Z") && start_ticks == expected)
}

async fn alive(worker: &WorkerIdentity) -> io::Result<bool> {
    match tokio::fs::read_to_string(format!("/proc/{}/stat", worker.pid)).await {
        Ok(stat) => matches_process(&stat, worker.start_ticks),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// PID reuse cannot extend a retired generation's lifetime. Unreadable process
/// state keeps the listener open and reports the error instead of losing work.
pub async fn wait(workers: &[WorkerIdentity]) {
    let mut previous_error = None;
    loop {
        let mut pending = false;
        for worker in workers {
            match alive(worker).await {
                Ok(value) => pending |= value,
                Err(error) => {
                    pending = true;
                    let message = format!("ingress worker {}: {error}", worker.pid);
                    if previous_error.as_ref() != Some(&message) {
                        tracing::error!(%message, "retirement cannot verify ingress drainage");
                        previous_error = Some(message);
                    }
                }
            }
        }
        if !pending {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(state: &str, start: u64) -> String {
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = state.to_owned();
        fields[19] = start.to_string();
        format!("123 (worker (with spaces)) {}", fields.join(" "))
    }

    #[test]
    fn process_identity_requires_the_original_live_process() {
        assert!(matches_process(&stat("S", 17), 17).unwrap());
        assert!(!matches_process(&stat("S", 18), 17).unwrap());
        assert!(!matches_process(&stat("Z", 17), 17).unwrap());
        assert!(matches_process("unreadable stat", 17).is_err());
        assert!(matches_process("1 (worker) S 0", 17).is_err());
    }

    #[test]
    fn malformed_retirement_identity_is_rejected() {
        let request = Retirement {
            ingress_workers: vec![WorkerIdentity {
                pid: 0,
                start_ticks: 17,
            }],
            successor_port: Some(1234),
        };
        assert!(request.validate().is_err());
        assert!(Retirement::default().validate().is_ok());
    }
}
