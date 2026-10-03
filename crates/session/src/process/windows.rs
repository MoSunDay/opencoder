//! A parent-owned Job Object and a start gate prevent unsupervised execution.
//! The helper waits for attachment before launching the actual command. If
//! the parent dies in that interval it exits without starting any workload.
use anyhow::{bail, ensure, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::process::Command;
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    System::{JobObjects::*, Threading::*},
};

static BINARY: OnceLock<PathBuf> = OnceLock::new();
fn jobs() -> &'static Mutex<Vec<Arc<OwnedHandle>>> {
    static JOBS: OnceLock<Mutex<Vec<Arc<OwnedHandle>>>> = OnceLock::new();
    JOBS.get_or_init(Mutex::default)
}
fn owned(handle: HANDLE) -> Result<OwnedHandle> {
    ensure!(
        !handle.is_null(),
        "Windows process handle failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}
fn wide(text: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(Some(0)).collect()
}

pub fn configure_supervisor_binary(path: PathBuf) -> Result<()> {
    let path = path.canonicalize()?;
    if let Some(existing) = BINARY.get() {
        ensure!(existing == &path, "process supervisor already configured");
        return Ok(());
    }
    BINARY
        .set(path)
        .map_err(|_| anyhow::anyhow!("process supervisor configuration raced"))
}

#[derive(Clone, Debug)]
pub struct RuncCleanup {
    pub root: PathBuf,
    pub id: String,
}
pub struct SpawnLease {
    job: Arc<OwnedHandle>,
    event: OwnedHandle,
}
pub struct OwnedSupervisor {
    job: Arc<OwnedHandle>,
    // The helper may not open the named gate until after spawned() returns.
    _event: OwnedHandle,
    armed: bool,
}
pub struct SignalTarget {
    job: Arc<OwnedHandle>,
}

pub fn command(program: impl AsRef<OsStr>) -> Result<Option<(Command, SpawnLease)>> {
    let binary = BINARY
        .get()
        .context("Windows process supervisor is not configured")?;
    let job = Arc::new(owned(unsafe {
        CreateJobObjectW(ptr::null(), ptr::null())
    })?);
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    ensure!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } != 0,
        "configure Windows process job: {}",
        std::io::Error::last_os_error()
    );
    let name = format!("Local\\OpenCoder-{}", ulid::Ulid::new());
    let event =
        owned(unsafe { CreateEventW(ptr::null(), 1, 0, wide(OsStr::new(&name)).as_ptr()) })?;
    let mut command = Command::new(binary);
    command
        .args([
            "internal-process-supervisor",
            "--",
            &name,
            &std::process::id().to_string(),
        ])
        .arg(program);
    command.creation_flags(CREATE_NO_WINDOW);
    Ok(Some((command, SpawnLease { job, event })))
}

impl SpawnLease {
    pub fn spawned(self, pid: Option<u32>) -> Result<OwnedSupervisor> {
        let process = owned(unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                0,
                pid.context("supervisor child has no PID")?,
            )
        })?;
        if unsafe { AssignProcessToJobObject(self.job.as_raw_handle(), process.as_raw_handle()) }
            == 0
        {
            let error = std::io::Error::last_os_error();
            unsafe {
                TerminateProcess(process.as_raw_handle(), 1);
            }
            return Err(error).context("attach Windows command to its job");
        }
        if unsafe { SetEvent(self.event.as_raw_handle()) } == 0 {
            unsafe {
                TerminateJobObject(self.job.as_raw_handle(), 1);
            }
            bail!(
                "release Windows command start gate: {}",
                std::io::Error::last_os_error()
            );
        }
        let mut tracked = jobs().lock().unwrap();
        tracked.retain(job_active);
        tracked.push(self.job.clone());
        Ok(OwnedSupervisor {
            job: self.job,
            _event: self.event,
            armed: true,
        })
    }
}
impl OwnedSupervisor {
    pub fn signal_target(&self) -> Result<SignalTarget> {
        Ok(SignalTarget {
            job: self.job.clone(),
        })
    }
    pub fn terminate(&mut self) {
        if self.armed {
            self.armed = false;
            unsafe {
                TerminateJobObject(self.job.as_raw_handle(), 1);
            }
        }
    }
}
impl Drop for OwnedSupervisor {
    fn drop(&mut self) {
        self.terminate();
    }
}
impl SignalTarget {
    pub fn terminate(&self) -> Result<()> {
        ensure!(
            unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } != 0,
            "terminate Windows job: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }
}

pub fn runc_command(
    _program: impl AsRef<OsStr>,
    _root: &Path,
    _id: &str,
) -> Result<Option<(Command, SpawnLease)>> {
    bail!("runc workloads require Linux")
}

pub fn supervisor_main(command: Vec<OsString>, cleanup: Option<RuncCleanup>) -> Result<i32> {
    ensure!(cleanup.is_none(), "runc workloads require Linux");
    ensure!(command.len() >= 3, "Windows supervisor arguments missing");
    let event =
        owned(unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, wide(&command[0]).as_ptr()) })?;
    let parent_pid: u32 = command[1].to_str().context("invalid parent PID")?.parse()?;
    let parent = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent_pid) })?;
    let handles = [parent.as_raw_handle(), event.as_raw_handle()];
    ensure!(
        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, 30_000) } == WAIT_OBJECT_0 + 1,
        "Windows supervisor parent exited or start gate timed out"
    );
    let status = std::process::Command::new(&command[2])
        .args(&command[3..])
        .status()?;
    Ok(status.code().unwrap_or(1))
}

pub fn active_owned_processes() -> usize {
    let mut jobs = jobs().lock().unwrap();
    jobs.retain(job_active);
    jobs.len()
}

// TerminateJobObject requests termination asynchronously. Keep a queryable
// parent-owned handle until the kernel confirms that every process has exited.
fn job_active(job: &Arc<OwnedHandle>) -> bool {
    let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        QueryInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
            ptr::null_mut(),
        )
    };
    ok == 0 || info.ActiveProcesses > 0
}
pub async fn wait_for_owned_processes(deadline: tokio::time::Instant) -> Result<()> {
    while active_owned_processes() > 0 {
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Windows process cleanup timed out"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Ok(())
}
