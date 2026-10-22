use clap::{CommandFactory, FromArgMatches};
use opencoder_cli::{
    cmd::ontology::{plan, OntologyCmd},
    Cli,
};
use opencoder_core::ontology::{platform_capabilities, CLI_DOMAINS};

fn parse(args: &[&str]) -> OntologyCmd {
    let mut argv = vec!["opencoder-cli", "ontology"];
    argv.extend(args);
    let matches = Cli::command().try_get_matches_from(argv).unwrap();
    OntologyCmd::from_arg_matches(matches.subcommand_matches("ontology").unwrap()).unwrap()
}

#[test]
fn ontology_commands_map_to_native_methods_paths_and_revisions() {
    let cases = [
        (vec!["session"], "GET", "/api/ontology/session"),
        (
            vec!["environments", "list"],
            "GET",
            "/api/ontology/environments",
        ),
        (
            vec!["environments", "create", "--json", "{}"],
            "POST",
            "/api/ontology/environments",
        ),
        (
            vec!["environments", "update", "other", "--json", "{}"],
            "PATCH",
            "/api/ontology/environments/other",
        ),
        (
            vec!["entity-types", "list"],
            "GET",
            "/api/ontology/envs/debug/entity-types",
        ),
        (
            vec!["entity-types", "create", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/entity-types",
        ),
        (
            vec!["entity-types", "update", "t", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/entity-types/t",
        ),
        (
            vec!["relationship-types", "list"],
            "GET",
            "/api/ontology/envs/debug/relationship-types",
        ),
        (
            vec!["relationship-types", "create", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/relationship-types",
        ),
        (
            vec!["relationship-types", "update", "r", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/relationship-types/r",
        ),
        (
            vec!["attributes", "list", "t"],
            "GET",
            "/api/ontology/envs/debug/entity-types/t/attributes",
        ),
        (
            vec!["attributes", "create", "t", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/entity-types/t/attributes",
        ),
        (
            vec!["attributes", "update", "1", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/attributes/1",
        ),
        (
            vec!["actions", "list", "t"],
            "GET",
            "/api/ontology/envs/debug/entity-types/t/actions",
        ),
        (
            vec!["actions", "create", "t", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/entity-types/t/actions",
        ),
        (
            vec!["actions", "update", "1", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/actions/1",
        ),
        (
            vec!["entities", "list"],
            "GET",
            "/api/ontology/envs/debug/entities",
        ),
        (
            vec!["entities", "get", "e"],
            "GET",
            "/api/ontology/envs/debug/entities/e",
        ),
        (
            vec!["entities", "create", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/entities",
        ),
        (
            vec!["entities", "update", "e", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/entities/e",
        ),
        (
            vec!["entities", "set-value", "e", "1", "--json", "{}"],
            "PUT",
            "/api/ontology/envs/debug/entities/e/attributes/1",
        ),
        (
            vec!["entities", "set-text", "e", "1", "--json", "{}"],
            "PUT",
            "/api/ontology/envs/debug/entities/e/attributes/1/text",
        ),
        (
            vec!["entities", "set-nfs-path", "e", "1", "--json", "{}"],
            "PUT",
            "/api/ontology/envs/debug/entities/e/attributes/1/nfs-path",
        ),
        (
            vec!["entities", "text-history", "e", "1"],
            "GET",
            "/api/ontology/envs/debug/entities/e/attributes/1/text",
        ),
        (
            vec!["entities", "text-get", "e", "1", "2"],
            "GET",
            "/api/ontology/envs/debug/entities/e/attributes/1/text/2",
        ),
        (
            vec!["relationships", "list"],
            "GET",
            "/api/ontology/envs/debug/relationships",
        ),
        (
            vec!["relationships", "create", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/relationships",
        ),
        (
            vec!["relationships", "update", "r", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/relationships/r",
        ),
        (
            vec!["directories", "tree"],
            "GET",
            "/api/ontology/envs/debug/directories/tree",
        ),
        (
            vec!["directories", "create", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/directories/tree",
        ),
        (
            vec!["directories", "update", "d", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/directories/d",
        ),
        (
            vec!["directories", "move", "d", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/directories/d/move",
        ),
        (
            vec!["aspects", "list"],
            "GET",
            "/api/ontology/envs/debug/graph-aspects",
        ),
        (
            vec!["aspects", "get", "a"],
            "GET",
            "/api/ontology/envs/debug/graph-aspects/a",
        ),
        (
            vec!["aspects", "create", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/graph-aspects",
        ),
        (
            vec!["aspects", "update", "a", "--json", "{}"],
            "PATCH",
            "/api/ontology/envs/debug/graph-aspects/a",
        ),
        (
            vec!["aspects", "delete", "a", "--expected-revision", "2"],
            "DELETE",
            "/api/ontology/envs/debug/graph-aspects/a",
        ),
        (vec!["graph"], "GET", "/api/ontology/envs/debug/graph"),
        (
            vec!["vectors", "get", "v"],
            "GET",
            "/api/ontology/envs/debug/vectors/v",
        ),
        (
            vec!["vectors", "set", "e", "1", "--json", "{}"],
            "PUT",
            "/api/ontology/envs/debug/entities/e/vectors/1",
        ),
        (
            vec!["vectors", "search", "--json", "{}"],
            "POST",
            "/api/ontology/envs/debug/vector-search",
        ),
        (
            vec!["capabilities", "list"],
            "GET",
            "/api/ontology/envs/debug/capabilities",
        ),
        (
            vec!["capabilities", "get", "c"],
            "GET",
            "/api/ontology/envs/debug/capabilities/c",
        ),
        (
            vec!["sync", "status"],
            "GET",
            "/api/ontology/envs/debug/sync",
        ),
        (
            vec!["sync", "check"],
            "GET",
            "/api/ontology/envs/debug/sync/check",
        ),
        (vec!["sync", "run"], "POST", "/api/ontology/envs/debug/sync"),
        (vec!["nfs", "status"], "GET", "/api/ontology/nfs"),
        (
            vec!["nfs", "set", "--json", "{}"],
            "POST",
            "/api/ontology/nfs",
        ),
    ];
    for (args, method, path) in cases {
        let request = plan(&parse(&args)).unwrap();
        assert_eq!(request.method.as_str(), method, "{args:?}");
        assert_eq!(request.path, path, "{args:?}");
        if args.contains(&"--json") {
            assert_eq!(request.body, Some(serde_json::json!({})));
        }
    }
    assert_eq!(
        plan(&parse(&[
            "aspects",
            "delete",
            "a",
            "--expected-revision",
            "9"
        ]))
        .unwrap()
        .query,
        vec![("expected_revision".into(), "9".into())]
    );
}

#[test]
fn queries_files_and_invalid_inputs_follow_cli_contract() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("body.json");
    std::fs::write(&path, r#"{"expected_revision":8,"is_deleted":true}"#).unwrap();
    let request = plan(&parse(&[
        "--env",
        "other",
        "entities",
        "update",
        "a/b",
        "--json",
        &format!("@{}", path.display()),
    ]))
    .unwrap();
    assert_eq!(request.path, "/api/ontology/envs/other/entities/a%2Fb");
    assert_eq!(request.body.unwrap()["expected_revision"], 8);
    let graph = plan(&parse(&[
        "graph",
        "--center",
        "a",
        "--center",
        "b",
        "--upstream-depth",
        "2",
    ]))
    .unwrap();
    assert_eq!(
        graph
            .query
            .iter()
            .filter(|(key, _)| key == "center")
            .count(),
        2
    );
    assert!(graph.query.contains(&("upstream_depth".into(), "2".into())));
    assert!(plan(&parse(&["entities", "create", "--json", "invalid"])).is_err());
    assert!(Cli::command()
        .try_get_matches_from(["cli", "ontology", "entities", "create"])
        .is_err());
    assert!(Cli::command()
        .try_get_matches_from(["cli", "ontology", "graph", "--depth", "256"])
        .is_err());
}

#[test]
fn every_public_cli_domain_is_registered_as_a_platform_capability() {
    let command = Cli::command();
    let entries = platform_capabilities();
    for sub in command
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
    {
        assert!(
            CLI_DOMAINS.contains(&sub.get_name()),
            "missing CLI domain {}",
            sub.get_name()
        );
        assert!(entries
            .iter()
            .any(|entry| entry.source_id == sub.get_name()));
    }
}
