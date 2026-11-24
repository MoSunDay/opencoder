//! Privilege transition shared by the sandbox init and the step relay:
//! drop the bounding set, switch ids, then apply the OCI capability sets.
//! Everything is computed before fork so the child only issues raw syscalls.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;

/// Capability numbers in kernel order; `CAP_CHECKPOINT_RESTORE` is the last
/// one Linux 6.8 defines.
const CAPS: &[&str] = &[
    "CAP_CHOWN",
    "CAP_DAC_OVERRIDE",
    "CAP_DAC_READ_SEARCH",
    "CAP_FOWNER",
    "CAP_FSETID",
    "CAP_KILL",
    "CAP_SETGID",
    "CAP_SETUID",
    "CAP_SETPCAP",
    "CAP_LINUX_IMMUTABLE",
    "CAP_NET_BIND_SERVICE",
    "CAP_NET_BROADCAST",
    "CAP_NET_ADMIN",
    "CAP_NET_RAW",
    "CAP_IPC_LOCK",
    "CAP_IPC_OWNER",
    "CAP_SYS_MODULE",
    "CAP_SYS_RAWIO",
    "CAP_SYS_CHROOT",
    "CAP_SYS_PTRACE",
    "CAP_SYS_PACCT",
    "CAP_SYS_ADMIN",
    "CAP_SYS_BOOT",
    "CAP_SYS_NICE",
    "CAP_SYS_RESOURCE",
    "CAP_SYS_TIME",
    "CAP_SYS_TTY_CONFIG",
    "CAP_MKNOD",
    "CAP_LEASE",
    "CAP_AUDIT_WRITE",
    "CAP_AUDIT_CONTROL",
    "CAP_SETFCAP",
    "CAP_MAC_OVERRIDE",
    "CAP_MAC_ADMIN",
    "CAP_SYSLOG",
    "CAP_WAKE_ALARM",
    "CAP_BLOCK_SUSPEND",
    "CAP_AUDIT_READ",
    "CAP_PERFMON",
    "CAP_BPF",
    "CAP_CHECKPOINT_RESTORE",
];
const LAST_CAP: u32 = CAPS.len() as u32 - 1;
const VERSION_3: u32 = 0x2008_0522;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Capabilities {
    pub bounding: BTreeSet<u32>,
    pub permitted: BTreeSet<u32>,
    pub effective: BTreeSet<u32>,
    pub inheritable: BTreeSet<u32>,
    pub ambient: BTreeSet<u32>,
}

impl Capabilities {
    /// OCI shape: five arrays of `CAP_*` names; absent arrays are empty.
    pub fn parse(value: Option<&Value>) -> Result<Self> {
        let empty = Value::Null;
        let value = value.unwrap_or(&empty);
        let read = |name: &str| -> Result<BTreeSet<u32>> {
            let Some(list) = value.get(name) else {
                return Ok(Default::default());
            };
            let list = list
                .as_array()
                .with_context(|| format!("capabilities.{name} must be an array"))?;
            list.iter()
                .map(|cap| {
                    cap.as_str()
                        .with_context(|| format!("capabilities.{name} must hold names"))?
                        .parse_cap()
                })
                .collect()
        };
        Ok(Self {
            bounding: read("bounding")?,
            permitted: read("permitted")?,
            effective: read("effective")?,
            inheritable: read("inheritable")?,
            ambient: read("ambient")?,
        })
    }
}

trait ParseCap {
    fn parse_cap(&self) -> Result<u32>;
}

impl ParseCap for str {
    fn parse_cap(&self) -> Result<u32> {
        CAPS.iter()
            .position(|known| *known == self)
            .map(|index| index as u32)
            .with_context(|| format!("unknown capability {self}"))
    }
}

/// The complete transition, computed on the node side of fork.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Transition {
    uid: u32,
    gid: u32,
    pub caps: Capabilities,
    pub no_new_privs: bool,
}

impl Transition {
    pub fn new(user: Option<&Value>, mut caps: Capabilities, no_new_privs: bool) -> Result<Self> {
        let empty = Value::Null;
        let user = user.unwrap_or(&empty);
        let id = |name: &str| -> Result<u32> {
            let id = user.get(name).and_then(Value::as_u64).unwrap_or(0);
            u32::try_from(id).context("process user id is out of range")
        };
        let (uid, gid) = (id("uid")?, id("gid")?);
        if uid != 0 {
            // Like runc: switching to a non-root user drops every permitted,
            // effective and ambient capability; only the bounding set is left
            // to restrict what the process could ever regain.
            caps.permitted.clear();
            caps.effective.clear();
            caps.ambient.clear();
        }
        ensure!(
            caps.ambient
                .iter()
                .all(|cap| caps.inheritable.contains(cap) && caps.permitted.contains(cap)),
            "ambient capabilities must be permitted and inheritable"
        );
        ensure!(
            caps.effective.iter().all(|cap| caps.permitted.contains(cap)),
            "effective capabilities must be permitted"
        );
        Ok(Self {
            uid,
            gid,
            caps,
            no_new_privs,
        })
    }

    /// Async-signal-safe: raw syscalls only, no allocation or locks. Returns
    /// the errno of the first failing step, or 0.
    pub unsafe fn apply(&self) -> i32 {
        for cap in 0..=LAST_CAP {
            if self.caps.bounding.contains(&cap) {
                continue;
            }
            let result = libc::prctl(libc::PR_CAPBSET_DROP, cap as libc::c_ulong, 0, 0, 0);
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
                return std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EINVAL);
            }
        }
        if libc::setgroups(0, std::ptr::null()) != 0
            || libc::setresgid(self.gid, self.gid, self.gid) != 0
            || libc::setresuid(self.uid, self.uid, self.uid) != 0
        {
            return std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EPERM);
        }
        let words = |set: &BTreeSet<u32>| -> [u32; 2] {
            let mut words = [0u32; 2];
            for cap in set {
                words[usize::from(*cap >= 32)] |= 1 << (cap % 32);
            }
            words
        };
        let effective = words(&self.caps.effective);
        let permitted = words(&self.caps.permitted);
        let inheritable = words(&self.caps.inheritable);
        let data = [
            Data { effective: effective[0], permitted: permitted[0], inheritable: inheritable[0] },
            Data { effective: effective[1], permitted: permitted[1], inheritable: inheritable[1] },
        ];
        let header = Header { version: VERSION_3, pid: 0 };
        if libc::syscall(libc::SYS_capset, &header, data.as_ptr()) != 0 {
            return std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EPERM);
        }
        for cap in &self.caps.ambient {
            if libc::prctl(libc::PR_CAP_AMBIENT_RAISE, *cap as libc::c_ulong, 0, 0, 0) != 0 {
                return std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EPERM);
            }
        }
        if self.no_new_privs && libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EPERM);
        }
        0
    }
}

#[repr(C)]
struct Header {
    version: u32,
    pid: i32,
}

#[repr(C)]
struct Data {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn caps() -> Capabilities {
        let filesystem = ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER"];
        Capabilities {
            bounding: filesystem.iter().map(|c| c.parse_cap().unwrap()).collect(),
            permitted: filesystem.iter().map(|c| c.parse_cap().unwrap()).collect(),
            effective: filesystem.iter().map(|c| c.parse_cap().unwrap()).collect(),
            inheritable: BTreeSet::new(),
            ambient: BTreeSet::new(),
        }
    }

    #[test]
    fn capabilities_parse_names_and_reject_garbage() {
        assert_eq!(
            Capabilities::parse(Some(&json!({"bounding":["CAP_CHOWN","CAP_SETFCAP"],"ambient":["CAP_BPF"]}))).unwrap(),
            Capabilities {
                bounding: BTreeSet::from([0, 31]),
                ambient: BTreeSet::from([39]),
                ..Default::default()
            }
        );
        assert_eq!(Capabilities::parse(None).unwrap(), Capabilities::default());
        assert!(Capabilities::parse(Some(&json!({"effective":"CAP_CHOWN"}))).is_err());
        assert!(Capabilities::parse(Some(&json!({"effective":["CAP_NOT_A_CAP"]}))).is_err());
        assert!("CAP_CHECKPOINT_RESTORE".parse_cap().unwrap() > 39);
    }

    #[test]
    fn transition_validates_capabilities_against_the_process_user() {
        let root = json!({"uid":0,"gid":65532});
        assert_eq!(
            Transition::new(Some(&root), caps(), true).unwrap(),
            Transition { uid: 0, gid: 65532, caps: caps(), no_new_privs: true }
        );
        // A non-root process keeps only the bounding restriction, like runc.
        let agent = json!({"uid":1000,"gid":65532});
        let transition = Transition::new(Some(&agent), caps(), false).unwrap();
        assert_eq!(transition.uid, 1000);
        assert_eq!(transition.caps.bounding, caps().bounding);
        assert!(transition.caps.permitted.is_empty());
        assert!(transition.caps.effective.is_empty());
        let none = Capabilities::default();
        assert_eq!(Transition::new(None, none.clone(), false).unwrap().uid, 0);
        let mut ambient = none.clone();
        ambient.ambient.insert(39);
        assert!(Transition::new(Some(&root), ambient.clone(), false).is_err());
        ambient.inheritable.insert(39);
        ambient.permitted.insert(39);
        ambient.effective.insert(39);
        assert!(Transition::new(Some(&root), ambient.clone(), false).is_ok());
        let mut stray = none;
        stray.effective.insert(1);
        assert!(Transition::new(Some(&root), stray, false).is_err());
    }
}
