//! Retain ownership while a killed container init finishes kernel teardown.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{io, path::Path, time::Duration};

#[derive(Deserialize)]
struct Init {
    id: String,
    init_process_pid: u32,
    init_process_start: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Process {
    start: u64,
    exiting: bool,
}

fn process(stat: &str, status: &str) -> Option<Process> {
    let tail: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    let flags: u64 = tail.get(6)?.parse().ok()?;
    let pending = status
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            matches!(name, "SigPnd" | "ShdPnd")
                .then(|| u64::from_str_radix(value.trim(), 16).ok())?
        })
        .fold(0, |all, value| all | value);
    Some(Process {
        start: tail.get(19)?.parse().ok()?,
        // PF_EXITING, or a pending SIGKILL while uninterruptible I/O ends.
        exiting: flags & 4 != 0 || pending & (1 << (libc::SIGKILL - 1)) != 0,
    })
}

fn read_process(pid: u32) -> Result<Option<Process>> {
    fn read(path: &str) -> Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    let Some(stat) = read(&format!("/proc/{pid}/stat"))? else {
        return Ok(None);
    };
    let Some(status) = read(&format!("/proc/{pid}/status"))? else {
        return Ok(None);
    };
    Ok(Some(
        process(&stat, &status).context("malformed container init process metadata")?,
    ))
}

async fn wait_exiting(
    expected_start: u64,
    mut read: impl FnMut() -> Result<Option<Process>>,
) -> Result<bool> {
    let Some(initial) = read()? else {
        return Ok(false);
    };
    if initial.start != expected_start || !initial.exiting {
        return Ok(false);
    }
    loop {
        tokio::time::sleep(Duration::from_millis(20)).await;
        match read()? {
            Some(current) if current.start == expected_start => {}
            _ => return Ok(true),
        }
    }
}

pub(super) async fn wait(state: &Path) -> Result<()> {
    let metadata = match std::fs::read(state.join("state.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let init: Init = serde_json::from_slice(&metadata).context("read owned runc init identity")?;
    ensure!(
        state.file_name().and_then(|name| name.to_str()) == Some(&init.id),
        "owned runc init identity mismatch"
    );
    if init.init_process_pid == 0 {
        return Ok(());
    }
    let pid = init.init_process_pid;
    if read_process(pid)?.is_some_and(|p| p.start == init.init_process_start && p.exiting) {
        tracing::warn!(pid, container = %init.id, "waiting for owned container init kernel teardown");
        wait_exiting(init.init_process_start, || read_process(pid)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exiting_init_is_owned_until_kernel_exit_not_command_deadline() {
        let (send, receive) = std::sync::mpsc::channel();
        let mut task = tokio::spawn(wait_exiting(42, move || {
            Ok(receive.try_recv().err().map(|_| Process {
                start: 42,
                exiting: true,
            }))
        }));
        assert!(tokio::time::timeout(Duration::from_millis(45), &mut task)
            .await
            .is_err());
        assert!(!task.is_finished());
        send.send(()).unwrap();
        assert!(task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn reused_pid_or_running_init_never_waits_or_signals() {
        for value in [
            None,
            Some(Process {
                start: 43,
                exiting: true,
            }),
            Some(Process {
                start: 42,
                exiting: false,
            }),
        ] {
            assert!(!wait_exiting(42, || Ok(value)).await.unwrap());
        }
        let mut calls = 0;
        assert!(wait_exiting(42, || {
            calls += 1;
            Ok(Some(Process {
                start: if calls == 1 { 42 } else { 43 },
                exiting: true,
            }))
        })
        .await
        .unwrap());
        assert_eq!(calls, 2);
    }

    #[test]
    fn kernel_exit_and_pending_kill_are_distinct_from_normal_io() {
        let stat = |flags: u64| {
            let mut fields = vec!["0".to_owned(); 20];
            fields[0] = "D".to_owned();
            fields[6] = flags.to_string();
            fields[19] = "42".to_owned();
            format!("42 (init (owned)) {}", fields.join(" "))
        };
        assert_eq!(
            process(&stat(4), "SigPnd:\t0000000000000000\n"),
            Some(Process {
                start: 42,
                exiting: true
            })
        );
        assert!(
            process(&stat(0), "ShdPnd:\t0000000000000100\n")
                .unwrap()
                .exiting
        );
        assert!(
            !process(&stat(0), "SigPnd:\t0000000000000000\n")
                .unwrap()
                .exiting
        );
        assert!(process("malformed", "").is_none());
    }

    #[tokio::test]
    async fn mismatched_container_identity_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("owned");
        std::fs::create_dir(&state).unwrap();
        std::fs::write(
            state.join("state.json"),
            br#"{"id":"foreign","init_process_pid":0,"init_process_start":0}"#,
        )
        .unwrap();
        assert!(wait(&state)
            .await
            .unwrap_err()
            .to_string()
            .contains("identity mismatch"));
        assert!(state.join("state.json").exists());
    }
}
