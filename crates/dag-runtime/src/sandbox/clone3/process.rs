//! The OCI `process` of a spec, shared by the init launcher and the step
//! relay: argv, env, cwd and the privilege transition applied before exec.
use super::privilege::{Capabilities, Transition};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::ffi::CString;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Process {
    pub argv: Vec<CString>,
    pub env: Vec<CString>,
    pub cwd: CString,
    pub transition: Transition,
}

impl Process {
    /// Name the lease descriptor for the init, replacing anything the bundle
    /// said about it so the init only trusts the launcher's entry.
    pub(super) fn lease(&mut self, fd: i32) {
        let prefix = format!("{}=", super::LEASE_ENV);
        self.env
            .retain(|entry| !entry.to_bytes().starts_with(prefix.as_bytes()));
        if let Ok(entry) = CString::new(format!("{prefix}{fd}")) {
            self.env.push(entry);
        }
    }

    pub fn parse(process: &Value) -> Result<Self> {
        let text = |value: &Value, what: &str| -> Result<Vec<CString>> {
            value
                .as_array()
                .with_context(|| format!("OCI process {what} missing"))?
                .iter()
                .map(|item| {
                    CString::new(
                        item.as_str()
                            .with_context(|| format!("OCI process {what} must be strings"))?,
                    )
                    .with_context(|| format!("OCI process {what} contains NUL"))
                })
                .collect()
        };
        let argv = text(&process["args"], "args")?;
        ensure!(!argv.is_empty(), "OCI process args must not be empty");
        let env = text(&process["env"], "env")?;
        let cwd = process["cwd"].as_str().context("OCI process cwd missing")?;
        ensure!(cwd.starts_with('/'), "OCI process cwd must be absolute");
        Ok(Self {
            transition: Transition::new(
                process.get("user"),
                Capabilities::parse(process.get("capabilities"))?,
                process["noNewPrivileges"].as_bool().unwrap_or(true),
            )?,
            cwd: CString::new(cwd).context("cwd contains NUL")?,
            argv,
            env,
        })
    }
}

/// NUL-terminated `execve` vectors over memory owned by `Process`, built
/// before fork so the child never allocates.
pub(super) fn pointers(items: &[CString]) -> Vec<*const libc::c_char> {
    items
        .iter()
        .map(|item| item.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn process() -> Value {
        json!({"args":["/usr/bin/dag-runner","step"],"env":["PATH=/usr/bin"],"cwd":"/workspace",
            "user":{"uid":1000,"gid":65532},"noNewPrivileges":true,
            "capabilities":{"bounding":["CAP_CHOWN","CAP_DAC_OVERRIDE","CAP_FOWNER"],
                "permitted":["CAP_CHOWN"],"effective":["CAP_CHOWN"]}})
    }

    #[test]
    fn lease_entry_names_the_descriptor_and_replaces_the_bundle_one() {
        let mut parsed = Process::parse(&process()).unwrap();
        parsed.lease(3);
        assert_eq!(parsed.env[1].to_bytes(), b"OPENCODER_SANDBOX_LEASE_FD=3");
        parsed
            .env
            .push(CString::new("OPENCODER_SANDBOX_LEASE_FD=9").unwrap());
        parsed.env.push(CString::new("KEEP=1").unwrap());
        parsed.lease(3);
        let names: Vec<_> = parsed
            .env
            .iter()
            .map(|entry| String::from_utf8_lossy(entry.to_bytes()).into_owned())
            .collect();
        assert_eq!(
            names,
            ["PATH=/usr/bin", "KEEP=1", "OPENCODER_SANDBOX_LEASE_FD=3"]
        );
    }

    #[test]
    fn process_parses_and_non_root_drops_capabilities() {
        let parsed = Process::parse(&process()).unwrap();
        assert_eq!(parsed.argv[0].to_bytes(), b"/usr/bin/dag-runner");
        assert_eq!(parsed.env[0].to_bytes(), b"PATH=/usr/bin");
        assert_eq!(parsed.cwd.to_bytes(), b"/workspace");
        assert!(parsed.transition.no_new_privs);
        assert!(parsed.transition.caps.bounding.contains(&0));
        assert!(parsed.transition.caps.effective.is_empty());
        assert_eq!(pointers(&parsed.argv).len(), 3);
        assert!(pointers(&parsed.argv)[2].is_null());
    }

    #[test]
    fn process_rejects_incomplete_specs() {
        let mut spec = process();
        spec["args"] = json!([]);
        assert!(Process::parse(&spec).unwrap_err().to_string().contains("empty"));
        spec["args"] = json!(["/bin/sh", 7]);
        assert!(Process::parse(&spec).is_err());
        spec["args"] = json!(["/bin/sh"]);
        spec["cwd"] = json!("workspace");
        assert!(Process::parse(&spec).unwrap_err().to_string().contains("absolute"));
        spec["cwd"] = json!("/w\u{0}ork");
        assert!(Process::parse(&spec).is_err());
        spec["cwd"] = json!("/");
        spec["capabilities"] = json!({"effective":["CAP_NOPE"]});
        assert!(Process::parse(&spec).is_err());
        spec["capabilities"] = json!({"effective":["CAP_KILL"]});
        // Non-root users lose the sets anyway; only the bounding list is read.
        assert!(Process::parse(&spec).is_ok());
        spec["user"] = json!({"uid":0,"gid":0});
        assert!(Process::parse(&spec).is_err());
        assert!(Process::parse(&json!({"args":["/bin/sh"],"env":[],"cwd":"/"})).is_ok());
    }
}
