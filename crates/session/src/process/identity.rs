//! Supervisor identity flags: build, parse and validate the privilege-drop
//! descriptor a node passes to the hidden Linux supervisor. Flags ride after
//! the node `--` separator (raw argv to the binary's CLI parser), followed by
//! a second `--` that separates them from the workload program.

use anyhow::{bail, Context, Result};
use opencoder_core::platform::RunAsIdentity;
use std::ffi::OsString;

pub(super) const SEPARATOR: &str = "--";
const UID_FLAG: &str = "--uid";
const GID_FLAG: &str = "--gid";
const GROUPS_FLAG: &str = "--groups";

/// Fail closed: a supervisor asked to drop to uid/gid 0 is a node bug.
pub(super) fn validate(identity: &RunAsIdentity) -> Result<()> {
    if identity.uid == 0 {
        bail!("supervisor identity uid 0 refused");
    }
    if identity.gid == 0 {
        bail!("supervisor identity gid 0 refused");
    }
    Ok(())
}

/// `--uid <u> --gid <g> --groups <g1,g2,…>`; `--groups` omitted when empty.
pub(super) fn flags(identity: &RunAsIdentity) -> Vec<OsString> {
    let mut flags = vec![
        OsString::from(UID_FLAG),
        OsString::from(identity.uid.to_string()),
        OsString::from(GID_FLAG),
        OsString::from(identity.gid.to_string()),
    ];
    if !identity.groups.is_empty() {
        let joined = identity
            .groups
            .iter()
            .map(|gid| gid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        flags.push(OsString::from(GROUPS_FLAG));
        flags.push(OsString::from(joined));
    }
    flags
}

/// Parse the leading identity flags of a supervisor argv. Returns the
/// identity (None when absent) and the remaining argv, which starts at the
/// identity separator when flags were present and at the program otherwise.
pub(super) fn parse(argv: &[OsString]) -> Result<(Option<RunAsIdentity>, &[OsString])> {
    let mut uid = None;
    let mut gid = None;
    let mut groups = None;
    let mut index = 0;
    while let Some(flag) = argv.get(index).and_then(|token| token.to_str()) {
        if !matches!(flag, UID_FLAG | GID_FLAG | GROUPS_FLAG) {
            break;
        }
        let value = argv
            .get(index + 1)
            .with_context(|| format!("supervisor {flag} missing value"))?;
        match flag {
            UID_FLAG => uid = Some(parse_id(flag, value)?),
            GID_FLAG => gid = Some(parse_id(flag, value)?),
            _ => groups = Some(parse_groups(value)?),
        }
        index += 2;
    }
    let identity = match (uid, gid) {
        (Some(uid), Some(gid)) => {
            let identity = RunAsIdentity {
                uid,
                gid,
                groups: groups.unwrap_or_default(),
                username: None,
            };
            validate(&identity)?;
            Some(identity)
        }
        (None, None) => None,
        _ => bail!("supervisor identity requires both {UID_FLAG} and {GID_FLAG}"),
    };
    let rest = if identity.is_some() {
        let (separator, tail) = argv[index..]
            .split_first()
            .context("supervised command missing")?;
        anyhow::ensure!(
            separator == SEPARATOR,
            "supervisor identity flags lack their workload separator"
        );
        tail
    } else {
        &argv[index..]
    };
    Ok((identity, rest))
}

fn parse_id(flag: &str, value: &OsString) -> Result<u32> {
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .with_context(|| format!("supervisor {flag} value invalid"))
}

fn parse_groups(value: &OsString) -> Result<Vec<u32>> {
    let text = value
        .to_str()
        .context("supervisor --groups value invalid")?;
    let mut groups = Vec::new();
    for group in text.split(',') {
        if group.is_empty() {
            bail!("supervisor --groups value invalid");
        }
        groups.push(group.parse().context("supervisor --groups value invalid")?);
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(uid: u32, gid: u32, groups: Vec<u32>) -> RunAsIdentity {
        RunAsIdentity {
            uid,
            gid,
            groups,
            username: Some("fixture".into()),
        }
    }

    #[test]
    fn flags_serialize_groups_and_omit_them_when_empty() {
        assert_eq!(
            flags(&identity(950, 951, vec![951, 960])),
            ["--uid", "950", "--gid", "951", "--groups", "951,960"].map(OsString::from)
        );
        assert_eq!(
            flags(&identity(950, 951, Vec::new())),
            ["--uid", "950", "--gid", "951"].map(OsString::from)
        );
    }

    #[test]
    fn parse_round_trips_flags_and_strips_them_from_argv() {
        let mut argv = flags(&identity(950, 951, vec![960]));
        argv.push(OsString::from(SEPARATOR));
        argv.push(OsString::from("bash"));
        argv.push(OsString::from("-c"));
        let (parsed, rest) = parse(&argv).unwrap();
        let parsed = parsed.expect("identity present");
        assert_eq!(parsed.uid, 950);
        assert_eq!(parsed.gid, 951);
        assert_eq!(parsed.groups, vec![960]);
        assert_eq!(rest, ["bash", "-c"].map(OsString::from));
    }

    #[test]
    fn parse_keeps_legacy_argv_without_flags_intact() {
        let argv = ["bash", "-c", "echo hi"].map(OsString::from);
        let (parsed, rest) = parse(&argv).unwrap();
        assert!(parsed.is_none());
        assert_eq!(rest, argv);
    }

    #[test]
    fn parse_refuses_root_identities_and_partial_flags() {
        for bad in [
            flags(&identity(0, 951, Vec::new())),
            flags(&identity(950, 0, Vec::new())),
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
        assert!(parse(&[OsString::from("--uid"), OsString::from("950")]).is_err());
        assert!(parse(&[
            OsString::from("--uid"),
            OsString::from("950"),
            OsString::from("--gid"),
            OsString::from("x")
        ])
        .is_err());
        assert!(
            parse(&[
                OsString::from("--uid"),
                OsString::from("950"),
                OsString::from("--gid"),
                OsString::from("951"),
                OsString::from("bash")
            ])
            .is_err(),
            "identity flags require the workload separator"
        );
    }
}
