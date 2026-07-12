//! Shared pty harness for the input-collector integration tests.
//!
//! Redirecting fd 0 (stdin) onto a pty is process-global, so each test that
//! does it must live in its own integration-test binary (one process per
//! file). This module factors the openpty + raw-mode + fd-0-redirect + Drop
//! cleanup so the characterization tests share one correct implementation.

#![cfg(unix)]

/// A pty pair wired to fd 0 so crossterm's `tty_fd()` (which picks fd 0 when
/// `isatty(0)` is true) reads the bytes we inject on the master end.
///
/// `Drop` restores the original stdin and closes the pty fds, so a failing
/// assertion never leaks the redirect.
pub(crate) struct PtyStdin {
    master: libc::c_int,
    slave: libc::c_int,
    saved_stdin: libc::c_int,
    orig_termios: libc::termios,
}

impl PtyStdin {
    /// Open a pty pair, put the slave into raw mode (so a lone Esc is readable
    /// without canonical line buffering), and redirect fd 0 onto the slave.
    pub(crate) fn open() -> Self {
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let r = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(r, 0, "openpty failed");

        let mut orig_termios: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::tcgetattr(slave, &mut orig_termios) },
            0,
            "tcgetattr failed"
        );
        let mut raw = orig_termios;
        unsafe { libc::cfmakeraw(&mut raw) };
        assert_eq!(
            unsafe { libc::tcsetattr(slave, libc::TCSANOW, &raw) },
            0,
            "tcsetattr(raw) failed"
        );

        let saved_stdin = unsafe { libc::dup(libc::STDIN_FILENO) };
        assert!(saved_stdin >= 0, "dup(stdin) failed");
        assert_eq!(
            unsafe { libc::dup2(slave, libc::STDIN_FILENO) },
            libc::STDIN_FILENO,
            "dup2(slave -> stdin) failed"
        );

        PtyStdin {
            master,
            slave,
            saved_stdin,
            orig_termios,
        }
    }

    /// Inject bytes on the pty master; crossterm reads them from fd 0 (slave).
    pub(crate) fn inject(&self, bytes: &[u8]) {
        let n = unsafe { libc::write(self.master, bytes.as_ptr() as *const _, bytes.len()) };
        assert_eq!(
            n,
            bytes.len() as libc::ssize_t,
            "write to pty master failed"
        );
    }
}

impl Drop for PtyStdin {
    fn drop(&mut self) {
        // Restore the real stdin first so a subsequent test (or the test
        // harness) talks to the original fd 0 again.
        unsafe {
            libc::dup2(self.saved_stdin, libc::STDIN_FILENO);
            libc::close(self.saved_stdin);
            libc::tcsetattr(self.slave, libc::TCSANOW, &self.orig_termios);
            libc::close(self.master);
            libc::close(self.slave);
        }
    }
}
