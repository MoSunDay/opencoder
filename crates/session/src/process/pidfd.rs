//! PID-reuse-safe signaling for owned supervisor and descendant processes.

use anyhow::{Context, Result};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub struct SignalTarget {
    fd: OwnedFd,
    pid: u32,
    start_time: u64,
}

impl SignalTarget {
    pub fn open(pid: u32) -> Result<Self> {
        let before = start_time(pid).context("owned process disappeared before pidfd open")?;
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as libc::c_int;
        if raw < 0 {
            return Err(std::io::Error::last_os_error())
                .context("pidfd_open required for safe process ownership");
        }
        let target = Self {
            fd: unsafe { OwnedFd::from_raw_fd(raw) },
            pid,
            start_time: before,
        };
        anyhow::ensure!(
            start_time(pid) == Some(before),
            "owned process identity changed during pidfd open"
        );
        Ok(target)
    }

    pub fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            fd: self.fd.try_clone()?,
            pid: self.pid,
            start_time: self.start_time,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn start_time(&self) -> u64 {
        self.start_time
    }

    pub fn signal(&self, signal: libc::c_int) -> Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(error.into())
    }
}

pub(crate) fn start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(") ")?.1;
    tail.split_whitespace().nth(19)?.parse().ok()
}
