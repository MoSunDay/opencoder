use super::tiers::{owned, profile_ownership};
use super::*;
use opencoder_core::config::{LinuxIdentityMode as Mode, OperatorAccessMode as Access};

fn identity() -> LinuxIdentity {
    LinuxIdentity {
        username: "ocop-operator-fixture".into(),
        uid: 940,
        gid: 940,
        groups: vec![940],
    }
}

#[test]
fn usernames_derive_from_execution_ids_within_the_linux_limits() {
    assert_eq!(
        username_for("operator-01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
        "ocop-operator-rrffq69g5fav"
    );
    // Longest kind prefix still fits the 32-byte limit.
    let long = username_for("maintenance-01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(long, "ocop-maintenance-rrffq69g5fav");
    assert!(long.len() <= MAX_USERNAME);
}

#[test]
fn username_derivation_fails_closed_on_malformed_ids() {
    for id in [
        "noulid",
        "operator-",
        "-01ARZ3NDEKTSV4RRFFQ69G5FAV",
        " OPERATOR-01A",
    ] {
        assert!(username_for(id).is_err(), "{id:?} must not derive a name");
    }
}

#[test]
fn username_validation_enforces_length_charset_and_first_letter() {
    assert!(valid_username("ocop-operator-rrffq69g5fav"));
    for bad in [
        "",
        "1ocop",
        "Ocop",
        "ocop_operator",
        "ocop operator",
        &"a".repeat(33),
    ] {
        assert!(!valid_username(bad), "{bad:?} must be invalid");
    }
    assert!(valid_username(&"a".repeat(32)));
}

#[test]
fn profile_ownership_maps_read_write_and_read_only() {
    let read_write = profile_ownership(Access::ReadWrite, &identity(), 0, 0);
    assert_eq!(read_write.home, owned(940, 940, 0o700));
    assert_eq!(read_write.workspace, read_write.home);
    assert_eq!(read_write.snapshot, owned(0, 0, 0o440));
    let read_only = profile_ownership(Access::ReadOnly, &identity(), 0, 0);
    assert_eq!(read_only.home, owned(0, 0, 0o555));
    assert_eq!(read_only.workspace, read_only.home);
    assert_eq!(read_only.snapshot, read_write.snapshot);
}

#[test]
fn admission_off_always_passes_and_enforced_requires_capability() {
    assert!(admission(Mode::Off, false).is_ok());
    assert!(admission(Mode::Off, true).is_ok());
    assert!(admission(Mode::Enforced, true).is_ok());
    let error = admission(Mode::Enforced, false).unwrap_err().to_string();
    assert!(error.contains("cannot provision"), "{error}");
}

#[test]
fn identity_converts_to_the_persistable_run_as_form() {
    let run_as = identity().run_as();
    assert_eq!(run_as.uid, 940);
    assert_eq!(run_as.groups, vec![940]);
    assert_eq!(run_as.username.as_deref(), Some("ocop-operator-fixture"));
}
