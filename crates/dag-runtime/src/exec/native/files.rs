use anyhow::{ensure, Result};
use std::{
    ffi::CString,
    fs::File,
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path},
};

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

fn relative(path: &Path) -> io::Result<CString> {
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace path must be confined",
        ));
    }
    CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn open(anchor: &File, path: &Path, directory: bool) -> io::Result<File> {
    let name = relative(path)?;
    let how = OpenHow {
        flags: (libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 }) as u64,
        mode: 0,
        resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS,
    };
    let descriptor = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            anchor.as_raw_fd(),
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(descriptor as i32) })
}

pub(crate) fn read(root: &Path, path: &Path) -> io::Result<File> {
    let file = open(&File::open(root)?, path, false)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace output must be a regular file",
        ));
    }
    Ok(file)
}

pub(crate) fn read_bounded(root: &Path, path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = read(root, path)?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "workspace output exceeds {limit} bytes"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "workspace output exceeds {limit} bytes"
    );
    Ok(bytes)
}

pub(crate) fn create_directory(root: &Path, path: &Path) -> Result<()> {
    relative(path)?;
    let mut anchor = File::open(root)?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            unreachable!()
        };
        let path = Path::new(name);
        anchor = match open(&anchor, path, true) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = relative(path)?;
                let result = unsafe { libc::mkdirat(anchor.as_raw_fd(), name.as_ptr(), 0o2770) };
                if result < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                    return Err(io::Error::last_os_error().into());
                }
                open(&anchor, path, true)?
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            unsafe { libc::fchown(anchor.as_raw_fd(), libc::geteuid(), 65532) } == 0,
            "workspace directory ownership failed"
        );
        ensure!(
            unsafe { libc::fchmod(anchor.as_raw_fd(), 0o2770) } == 0,
            "workspace directory permissions failed"
        );
    }
    Ok(())
}
