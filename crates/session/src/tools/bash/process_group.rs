//! Ownership guard for one bash tool process group.

use super::super::bg;

/// Owns the private process group created for one bash tool invocation.
///
/// Dropping the tool future drops this guard, so cancellation cannot bypass
/// descendant termination or leave the process visible in the global `/ps`
/// registry. Timeout handoff moves the guard into the background supervisor.
pub(crate) struct ProcessGroupGuard {
    pid: u32,
    pgid: libc::pid_t,
    supervisor: Option<crate::process::OwnedSupervisor>,
    armed: bool,
}

impl ProcessGroupGuard {
    pub(crate) fn is_supervised(&self) -> bool {
        self.supervisor.is_some()
    }

    pub(crate) fn registered(pid: u32, pgid: libc::pid_t, session_id: String) -> Self {
        bg::register(pid, pgid, session_id);
        Self {
            pid,
            pgid,
            supervisor: None,
            armed: true,
        }
    }

    pub(crate) fn registered_supervised(
        pid: u32,
        session_id: String,
        supervisor: crate::process::OwnedSupervisor,
    ) -> anyhow::Result<Self> {
        let signal = supervisor.signal_target()?;
        bg::register_supervised(pid, signal, session_id);
        Ok(Self {
            pid,
            pgid: pid as libc::pid_t,
            supervisor: Some(supervisor),
            armed: true,
        })
    }

    /// Terminate descendants and release the registry entry exactly once.
    pub(crate) fn terminate(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.terminate();
            bg::unregister(self.pid);
        } else {
            // SAFETY: direct bash starts with `setsid`, so its pid is a
            // private process group id. ESRCH means it already exited.
            unsafe {
                let _ = libc::kill(-self.pgid, libc::SIGKILL);
            }
            bg::unregister_group(self.pid, self.pgid);
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}
