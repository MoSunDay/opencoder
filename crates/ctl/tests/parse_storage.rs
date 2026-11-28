//! Parse-level contract for `opencoder-cli storage`: the SQL bridge is
//! read-only unless `--writable` is given, the two modes are mutually
//! exclusive, and the TRUNCATE checkpoint still requires the explicit
//! stopped-writers acknowledgement.

use clap::Parser;
use opencoder_cli::maintenance::StorageAction;

fn storage(args: &[&str]) -> Result<StorageAction, clap::Error> {
    #[derive(Parser)]
    struct Wrap {
        #[command(subcommand)]
        cmd: StorageAction,
    }
    Wrap::try_parse_from(std::iter::once("cli").chain(args.iter().copied()))
        .map(|wrapped| wrapped.cmd)
}

/// Whether the bridge would open the database writable.
fn writable(action: &StorageAction) -> bool {
    match action {
        StorageAction::Serve { writable, .. } => *writable,
        _ => panic!("not a serve command"),
    }
}

#[test]
fn serve_is_read_only_by_default() {
    let action = storage(&["serve", "--database", "x.db"]).unwrap();
    assert!(!writable(&action));
}

#[test]
fn serve_accepts_the_legacy_read_only_flag_as_a_no_op() {
    let action = storage(&["serve", "--database", "x.db", "--read-only"]).unwrap();
    assert!(!writable(&action));
}

#[test]
fn serve_opting_into_writes_takes_the_writer_side() {
    let action = storage(&["serve", "--database", "x.db", "--writable"]).unwrap();
    assert!(writable(&action));
}

#[test]
fn serve_rejects_contradicting_modes() {
    let error = storage(&["serve", "--database", "x.db", "--read-only", "--writable"])
        .expect_err("read-only and writable cannot be combined");
    assert!(error.to_string().contains("cannot be used with"));
}

#[test]
fn checkpoint_requires_the_stopped_writers_acknowledgement() {
    let refused = storage(&["checkpoint", "--database", "x.db"])
        .expect_err("checkpoint without --writers-stopped must not parse");
    assert!(refused.to_string().contains("--writers-stopped"));
    let accepted = storage(&["checkpoint", "--database", "x.db", "--writers-stopped"]).unwrap();
    assert!(matches!(accepted, StorageAction::Checkpoint { .. }));
}
