//! Parsing + plan-mapping tests for the `schedule` domain (rules/01: every
//! subcommand has a case). Parsing goes through the real `Cli` surface;
//! plan assertions pin method, path, query mapping and body passing — no
//! network involved.

use clap::{CommandFactory, FromArgMatches};
use opencoder_cli::cmd::schedule::{plan as schedule_plan, ScheduleCmd};
use opencoder_cli::{Cli, Command};
use reqwest::Method;

/// Try parsing `opencoder-cli <args...>` into raw matches (Err on bad argv).
fn try_matches_of(args: &[&str]) -> Result<clap::ArgMatches, clap::Error> {
    let argv: Vec<String> = std::iter::once("opencoder-cli")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    Cli::command().try_get_matches_from(argv)
}

fn matches_of(args: &[&str]) -> clap::ArgMatches {
    try_matches_of(args).expect("valid cli invocation")
}

/// Parse `opencoder-cli <args...>` and unwrap the typed `schedule` subcommand.
fn schedule(args: &[&str]) -> ScheduleCmd {
    ScheduleCmd::from_arg_matches(matches_of(args).subcommand_matches("schedule").unwrap()).unwrap()
}

/// Assert a plan's full contract: method, path, query pairs, body.
fn assert_plan(
    plan: &opencoder_cli::http::RequestPlan,
    method: Method,
    path: &str,
    query: &[(&str, &str)],
    body: Option<serde_json::Value>,
) {
    assert_eq!(plan.method, method, "method for {path}");
    assert_eq!(plan.path, path, "path");
    assert_eq!(
        plan.query,
        query
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>(),
        "query for {path}"
    );
    assert_eq!(plan.body, body, "body for {path}");
}

#[test]
fn list_maps_to_the_schedules_route() {
    assert_plan(
        &schedule_plan(&schedule(&["schedule", "list"])).unwrap(),
        Method::GET,
        "/api/schedules",
        &[],
        None,
    );
}

#[test]
fn ls_alias_resolves_to_list() {
    assert!(matches!(schedule(&["schedule", "ls"]), ScheduleCmd::List));
}

#[test]
fn runs_maps_to_the_fire_history_route() {
    assert_plan(
        &schedule_plan(&schedule(&["schedule", "runs", "nightly-brain"])).unwrap(),
        Method::GET,
        "/api/schedules/nightly-brain/runs",
        &[],
        None,
    );
    // Ids with path separators must survive verbatim (no re-encoding).
    assert_plan(
        &schedule_plan(&schedule(&["schedule", "runs", "team/eu", "--limit", "7"])).unwrap(),
        Method::GET,
        "/api/schedules/team/eu/runs",
        &[("limit", "7")],
        None,
    );
}

#[test]
fn runs_limit_defaults_to_absent_and_rejects_non_numeric() {
    assert!(matches!(
        schedule(&["schedule", "runs", "a"]),
        ScheduleCmd::Runs { limit: None, .. }
    ));
    assert!(try_matches_of(&["schedule", "runs", "x", "--limit", "abc"]).is_err());
    assert!(try_matches_of(&["schedule", "runs", "x", "--limit", "0"]).is_ok());
}

#[test]
fn schedule_routes_through_the_cli_dispatch() {
    for argv in [
        vec!["schedule", "list"],
        vec!["schedule", "ls"],
        vec!["schedule", "runs", "s1", "--limit", "5"],
    ] {
        let matches = matches_of(&argv);
        let (name, _) = matches.subcommand().expect("schedule subcommand present");
        assert_eq!(name, "schedule");
    }
    // The top-level Command enum must carry the schedule arm (dispatch wiring
    // in lib.rs maps it to cmd::schedule::run).
    let dispatch = Cli::command()
        .try_get_matches_from(["opencoder-cli", "schedule", "list"])
        .unwrap();
    assert!(matches!(
        Command::from_arg_matches(&dispatch).unwrap(),
        Command::Schedule(ScheduleCmd::List)
    ));
}
