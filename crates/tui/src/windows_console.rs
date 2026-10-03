//! Preserve the caller's console modes, including console close events.
use anyhow::{Context, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleCtrlHandler, SetConsoleMode, CTRL_BREAK_EVENT,
        CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    },
};

static MODES: Mutex<Option<[(isize, u32); 2]>> = Mutex::new(None);
static ARMED: AtomicBool = AtomicBool::new(false);

pub(crate) fn capture() -> Result<()> {
    let mut saved = MODES.lock().unwrap();
    if saved.is_some() {
        return Ok(());
    }
    let mut modes = [(0, 0); 2];
    for (index, id) in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE]
        .into_iter()
        .enumerate()
    {
        let handle = unsafe { GetStdHandle(id) };
        let mut mode = 0;
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return Err(std::io::Error::last_os_error())
                .context("TUI requires an interactive Windows console");
        }
        modes[index] = (handle as isize, mode);
    }
    if unsafe {
        SetConsoleMode(
            modes[1].0 as HANDLE,
            modes[1].1 | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("enable Windows terminal output");
    }
    *saved = Some(modes);
    drop(saved);
    arm_once();
    Ok(())
}

pub(crate) fn restore_modes() {
    if let Some(modes) = MODES.lock().unwrap().take() {
        for (handle, mode) in modes {
            unsafe {
                SetConsoleMode(handle as HANDLE, mode);
            }
        }
    }
}

pub(crate) fn arm_once() {
    if !ARMED.swap(true, Ordering::AcqRel)
        && unsafe { SetConsoleCtrlHandler(Some(control), 1) } == 0
    {
        ARMED.store(false, Ordering::Release);
        tracing::warn!("cannot register Windows console exit handler");
    }
}

unsafe extern "system" fn control(event: u32) -> i32 {
    if matches!(
        event,
        CTRL_C_EVENT
            | CTRL_BREAK_EVENT
            | CTRL_CLOSE_EVENT
            | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT
    ) {
        crate::terminal::TerminalGuard::restore();
    }
    // The OS's default handler completes termination and closes owned jobs.
    0
}
