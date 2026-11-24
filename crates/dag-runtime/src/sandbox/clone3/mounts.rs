//! OCI `mounts` entries applied with mount(2) inside the launcher's private
//! mount namespace. The supported kinds are exactly the ones `run::config`
//! renders: bind, tmpfs, proc and devpts. Unknown options fail closed.
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
use std::{
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Bind,
    Tmpfs,
    Proc,
    Devpts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Mount {
    pub kind: Kind,
    /// Host source of a bind mount.
    pub source: PathBuf,
    /// Guest path without the leading slash; empty for `/`.
    pub destination: PathBuf,
    pub recursive: bool,
    pub readonly: bool,
    /// `MS_NOSUID | MS_NODEV | MS_NOEXEC`, as requested by the options.
    pub flags: u64,
    /// Filesystem options in order: `key=value` pairs and bare words.
    pub data: Vec<String>,
}

impl Mount {
    fn parse(entry: &Value) -> Result<Self> {
        let destination = normalized(
            entry["destination"]
                .as_str()
                .context("mount destination missing")?,
        )?;
        let kind = match entry["type"].as_str().context("mount type missing")? {
            "bind" => Kind::Bind,
            "tmpfs" => Kind::Tmpfs,
            "proc" => Kind::Proc,
            "devpts" => Kind::Devpts,
            other => bail!("unsupported OCI mount type {other}"),
        };
        let source = match &entry["source"] {
            Value::String(source) => PathBuf::from(source),
            _ => PathBuf::new(),
        };
        ensure!(
            kind != Kind::Bind || source.is_absolute(),
            "bind mount source must be absolute: {}",
            destination.display()
        );
        let mut mount = Self {
            kind,
            source,
            destination,
            recursive: false,
            readonly: false,
            flags: 0,
            data: Vec::new(),
        };
        let options = entry["options"].as_array().cloned().unwrap_or_default();
        for option in options.iter().filter_map(Value::as_str) {
            mount.option(option)?;
        }
        ensure!(
            !(mount.kind == Kind::Proc && mount.readonly),
            "read-only proc is not supported"
        );
        Ok(mount)
    }

    fn option(&mut self, option: &str) -> Result<()> {
        match option {
            "ro" => self.readonly = true,
            "rw" => self.readonly = false,
            "rbind" => self.recursive = true,
            "bind" => self.recursive = false,
            "nosuid" => self.flags |= libc::MS_NOSUID,
            "nodev" => self.flags |= libc::MS_NODEV,
            "noexec" => self.flags |= libc::MS_NOEXEC,
            "suid" | "dev" | "exec" => {}
            "rprivate" | "private" | "rslave" | "slave" | "rshared" | "shared"
            | "runbindable" | "unbindable" => {}
            // Filesystem options reach the kernel as mount data; a bind mount
            // ignores them, so for binds every unknown word fails closed.
            _ => {
                let word = !option.is_empty()
                    && option.bytes().all(|b| b.is_ascii_graphic() && b != b',');
                if self.kind == Kind::Bind || !word {
                    bail!("unsupported OCI mount option {option:?}");
                }
                self.data.push(option.to_owned());
            }
        }
        Ok(())
    }

    fn data(&self) -> Result<CString> {
        CString::new(self.data.join(",")).context("mount data contains NUL")
    }
}

pub(super) fn parse_all(config: &Value) -> Result<Vec<Mount>> {
    config["mounts"]
        .as_array()
        .context("OCI mounts missing")?
        .iter()
        .map(Mount::parse)
        .collect()
}

/// Guest destinations become host paths under `prefix`; the launcher prefixes
/// the run's rootfs overlay, the chrooted child passes `/`.
pub(super) fn target(prefix: &Path, destination: &Path) -> Result<CString> {
    let path = if destination.as_os_str().is_empty() {
        prefix.to_path_buf()
    } else {
        prefix.join(destination)
    };
    CString::new(path.as_os_str().as_bytes()).context("mount target contains NUL")
}

/// Mount one entry at its prefixed target. `proc` must be mounted from inside
/// the target PID namespace, so it is refused here and applied by the child.
pub(super) fn apply(prefix: &Path, mount: &Mount) -> Result<()> {
    ensure!(mount.kind != Kind::Proc, "proc belongs to the sandbox init");
    let target = target(prefix, &mount.destination)?;
    let source = CString::new(mount.source.as_os_str().as_bytes())
        .context("mount source contains NUL")?;
    let data = mount.data()?;
    let data = if data.to_bytes().is_empty() {
        std::ptr::null()
    } else {
        data.as_ptr().cast::<libc::c_void>()
    };
    // Like runc, missing mountpoints are created inside the private mount
    // namespace; a source that is not a directory keeps its target untouched.
    let path = prefix.join(&mount.destination);
    let directory = matches!(mount.kind, Kind::Tmpfs | Kind::Devpts) || mount.source.is_dir();
    if directory && !path.exists() {
        std::fs::create_dir_all(&path)
            .with_context(|| format!("create mountpoint {}", mount.destination.display()))?;
    }
    let failure = |what: &str| {
        anyhow::Error::from(std::io::Error::last_os_error())
            .context(format!("{what} mount {}", mount.destination.display()))
    };
    let result = unsafe {
        match mount.kind {
            Kind::Bind => {
                let mut flags = libc::MS_BIND | if mount.recursive { libc::MS_REC } else { 0 };
                let result = libc::mount(
                    source.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    flags,
                    std::ptr::null(),
                );
                if result != 0 {
                    return Err(failure("bind"));
                }
                // Like runc: a bind first, then one remount carrying the
                // protection flags. The remount reaches only the top mount.
                flags |= libc::MS_REMOUNT
                    | mount.flags
                    | if mount.readonly { libc::MS_RDONLY } else { 0 };
                libc::mount(std::ptr::null(), target.as_ptr(), std::ptr::null(), flags, std::ptr::null())
            }
            Kind::Tmpfs => libc::mount(
                c"tmpfs".as_ptr(),
                target.as_ptr(),
                c"tmpfs".as_ptr(),
                mount.flags | if mount.readonly { libc::MS_RDONLY } else { 0 },
                data,
            ),
            Kind::Devpts => libc::mount(
                c"devpts".as_ptr(),
                target.as_ptr(),
                c"devpts".as_ptr(),
                mount.flags,
                data,
            ),
            Kind::Proc => unreachable!("proc is refused above"),
        }
    };
    if result != 0 {
        return Err(failure(match mount.kind {
            Kind::Tmpfs => "tmpfs",
            Kind::Devpts => "devpts",
            _ => "bind",
        }));
    }
    Ok(())
}

/// `root.readonly`: the rootfs becomes a read-only bind inside the private
/// mount namespace; the mounts applied afterwards stay writable, as with runc.
pub(super) fn readonly_root(rootfs: &Path) -> Result<()> {
    let target = target(rootfs, Path::new(""))?;
    let flags = libc::MS_BIND
        | libc::MS_REMOUNT
        | libc::MS_RDONLY
        | libc::MS_NODEV
        | libc::MS_NOSUID;
    unsafe {
        ensure!(
            libc::mount(target.as_ptr(), target.as_ptr(), std::ptr::null(), libc::MS_BIND, std::ptr::null()) == 0,
            "bind sandbox rootfs: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            libc::mount(std::ptr::null(), target.as_ptr(), std::ptr::null(), flags, std::ptr::null()) == 0,
            "remount sandbox rootfs read-only: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

/// runc's default devices on a tmpfs `/dev`, bound from the node, plus the
/// standard fd symlinks. Called right after the `/dev` entry is mounted.
pub(super) fn devices(prefix: &Path) -> Result<()> {
    let dev = prefix.join("dev");
    for name in ["null", "zero", "full", "random", "urandom", "tty"] {
        let source = Path::new("/dev").join(name);
        if !source.exists() {
            continue;
        }
        let target = dev.join(name);
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&target)
            .with_context(|| format!("prepare /dev/{name}"))?;
        let source = CString::new(source.as_os_str().as_bytes())?;
        let point = CString::new(target.as_os_str().as_bytes())?;
        let result = unsafe {
            libc::mount(source.as_ptr(), point.as_ptr(), std::ptr::null(), libc::MS_BIND, std::ptr::null())
        };
        ensure!(result == 0, "bind /dev/{name}: {}", std::io::Error::last_os_error());
    }
    for (link, original) in [
        // The devpts instance's own ptmx: a 5:2 node opened outside the
        // instance does not find it, like runc the image links to it.
        ("ptmx", "pts/ptmx"),
        ("fd", "/proc/self/fd"),
        ("stdin", "/proc/self/fd/0"),
        ("stdout", "/proc/self/fd/1"),
        ("stderr", "/proc/self/fd/2"),
    ] {
        match std::os::unix::fs::symlink(original, dev.join(link)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("create /dev fd symlink"),
        }
    }
    Ok(())
}

fn normalized(destination: &str) -> Result<PathBuf> {
    let path = Path::new(destination);
    ensure!(path.is_absolute(), "mount destination must be absolute");
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => clean.push(part),
            _ => bail!("mount destination must be normalized: {destination}"),
        }
    }
    Ok(clean)
}

/// `maskedPaths` (bind `/dev/null` over them) and `readonlyPaths` (read-only
/// self binds) from the OCI config, resolved as guest paths for the chrooted
/// child. Missing files are skipped, as runc does.
pub(super) struct Protection {
    masked: Vec<CString>,
    readonly: Vec<CString>,
}

pub(super) fn protection(config: &Value) -> Result<Protection> {
    let collect = |name: &str| -> Result<Vec<CString>> {
        config["linux"][name]
            .as_array()
            .map(|paths| {
                paths
                    .iter()
                    .map(|path| {
                        let path = path.as_str().context("path lists hold strings")?;
                        let guest = normalized(path)?;
                        CString::new(Path::new("/").join(guest).as_os_str().as_bytes())
                            .context("path contains NUL")
                    })
                    .collect()
            })
            .transpose()
            .map(|paths| paths.unwrap_or_default())
    };
    Ok(Protection {
        masked: collect("maskedPaths")?,
        readonly: collect("readonlyPaths")?,
    })
}

/// Async-signal-safe (the sandbox init calls it after its `chroot`).
pub(super) unsafe fn protect(protection: &Protection) -> i32 {
    let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EPERM);
    for path in &protection.masked {
        if libc::access(path.as_ptr(), libc::F_OK) != 0 {
            continue;
        }
        if libc::mount(c"/dev/null".as_ptr(), path.as_ptr(), std::ptr::null(), libc::MS_BIND, std::ptr::null()) != 0 {
            return errno();
        }
    }
    for path in &protection.readonly {
        if libc::access(path.as_ptr(), libc::F_OK) != 0 {
            continue;
        }
        let bind = libc::MS_BIND | libc::MS_REC;
        let remount = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY;
        if libc::mount(path.as_ptr(), path.as_ptr(), std::ptr::null(), bind, std::ptr::null()) != 0
            || libc::mount(std::ptr::null(), path.as_ptr(), std::ptr::null(), remount, std::ptr::null()) != 0
        {
            return errno();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oci_entries_parse_into_mounts_and_unknown_options_fail() {
        let config = json!({"mounts":[
            {"type":"bind","source":"/run/x","destination":"/workspace","options":["bind","rw","nosuid","nodev"]},
            {"type":"bind","source":"/k","destination":"/run/opencoder/knowledge","options":["rbind","ro","nosuid","nodev"]},
            {"type":"tmpfs","source":"tmpfs","destination":"/tmp","options":["rw","nosuid","nodev","noexec","mode=1777","size=256m"]},
            {"type":"devpts","source":"devpts","destination":"/dev/pts","options":["rw","nosuid","noexec","newinstance","ptmxmode=0666","mode=0620","gid=5"]},
            {"type":"proc","source":"proc","destination":"/proc","options":["nosuid","nodev","noexec"]}
        ]});
        let mounts = parse_all(&config).unwrap();
        assert_eq!(mounts.len(), 5);
        assert_eq!(mounts[0], Mount { kind: Kind::Bind, source: PathBuf::from("/run/x"), destination: PathBuf::from("workspace"),
            recursive: false, readonly: false, flags: libc::MS_NOSUID | libc::MS_NODEV, data: vec![] });
        assert!(mounts[1].recursive && mounts[1].readonly);
        assert_eq!(mounts[2].flags, libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC);
        assert_eq!(mounts[2].data, ["mode=1777", "size=256m"]);
        assert_eq!(mounts[3].data, ["newinstance", "ptmxmode=0666", "mode=0620", "gid=5"]);
        assert_eq!(mounts[4].kind, Kind::Proc);
        assert!(parse_all(&json!({"mounts":[{"type":"bind","destination":"/w","options":["bind"]}]})).is_err());
        assert!(parse_all(&json!({"mounts":[{"type":"overlay","destination":"/w"}]})).is_err());
        assert!(parse_all(&json!({"mounts":[{"type":"bind","source":"/a","destination":"/w","options":["rshared"]}]})).is_ok());
        assert!(parse_all(&json!({"mounts":[{"type":"bind","source":"/a","destination":"/w","options":["idmap"]}]})).is_err());
        assert!(parse_all(&json!({"mounts":[{"type":"proc","destination":"/proc","options":["ro"]}]})).is_err());
        assert!(parse_all(&json!({"mounts":[]})).is_ok());
        assert!(parse_all(&json!({})).is_err());
    }

    #[test]
    fn destinations_are_absolute_normalized_guest_paths() {
        assert_eq!(normalized("/").unwrap(), PathBuf::new());
        assert_eq!(normalized("/run/opencoder-private/exec").unwrap(), PathBuf::from("run/opencoder-private/exec"));
        assert_eq!(normalized("/a//b/").unwrap(), PathBuf::from("a/b"));
        assert!(normalized("relative").is_err());
        assert!(normalized("/a/../b").is_err());
        assert_eq!(normalized("/a/./b").unwrap(), PathBuf::from("a/b"));
        assert_eq!(target(Path::new("/r"), &normalized("/proc/sys").unwrap()).unwrap().to_bytes(), b"/r/proc/sys");
        assert_eq!(target(Path::new("/r"), &normalized("/").unwrap()).unwrap().to_bytes(), b"/r");
    }

    #[test]
    fn protection_lists_resolve_to_guest_paths() {
        let config = json!({"linux":{"maskedPaths":["/proc/kcore","/proc/keys"],"readonlyPaths":["/proc/sys","/proc/sysrq-trigger"]}});
        let applied = protection(&config).unwrap();
        assert_eq!(applied.masked[1].to_bytes(), b"/proc/keys");
        assert_eq!(applied.readonly[0].to_bytes(), b"/proc/sys");
        let empty = protection(&json!({})).unwrap();
        assert!(empty.masked.is_empty() && empty.readonly.is_empty());
        assert!(protection(&json!({"linux":{"readonlyPaths":["/proc/../x"]}})).is_err());
        assert!(protection(&json!({"linux":{"maskedPaths":[1]}})).is_err());
        assert!(protection(&json!({"maskedPaths":["/proc/kcore"]})).unwrap().masked.is_empty());
    }
}
