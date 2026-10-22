use clap::Parser;
use opencoder_cli::cmd::project::{self, ProjectCmd};
use serde_json::json;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: ProjectCmd,
}

#[test]
fn comment_commands_parse_pagination_encode_ids_and_read_json_files() {
    let args = Args::try_parse_from([
        "ctl",
        "todos",
        "comments",
        "list",
        "a/b",
        "--after-seq",
        "7",
        "--limit",
        "50",
    ])
    .unwrap();
    let plan = project::plan(&args.command).unwrap();
    assert_eq!(plan.path, "/api/project/todos/a%2Fb/comments");
    assert_eq!(
        plan.query,
        [
            ("after_seq".into(), "7".into()),
            ("limit".into(), "50".into())
        ]
    );
    assert!(Args::try_parse_from([
        "ctl",
        "todos",
        "comments",
        "list",
        "t",
        "--after-seq",
        "7",
        "--before-seq",
        "3"
    ])
    .is_err());
    let invalid =
        Args::try_parse_from(["ctl", "todos", "comments", "list", "t", "--limit", "101"]).unwrap();
    assert!(project::plan(&invalid.command).is_err());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("comment.json");
    let body =
        json!({"request_id":"stable","body_md":"**note**","capability_id":"builtin-operator"});
    std::fs::write(&file, body.to_string()).unwrap();
    let args = Args::try_parse_from([
        "ctl",
        "todos",
        "comments",
        "add",
        "t",
        "--json",
        &format!("@{}", file.display()),
    ])
    .unwrap();
    let plan = project::plan(&args.command).unwrap();
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.body.unwrap(), body);
    assert!(Args::try_parse_from(["ctl", "todos", "comments", "add", "t"]).is_err());
}

#[test]
fn comment_mutation_commands_encode_scope_and_require_delete_revision() {
    let args = Args::try_parse_from([
        "ctl",
        "todos",
        "comments",
        "edit",
        "a/b",
        "12",
        "--json",
        r#"{"body_md":"merged","expected_revision":2}"#,
    ])
    .unwrap();
    let plan = project::plan(&args.command).unwrap();
    assert_eq!(plan.method, reqwest::Method::PATCH);
    assert_eq!(plan.path, "/api/project/todos/a%2Fb/comments/12");
    assert_eq!(plan.body.unwrap()["expected_revision"], 2);
    let args = Args::try_parse_from([
        "ctl",
        "todos",
        "comments",
        "delete",
        "a/b",
        "12",
        "--expected-revision",
        "2",
    ])
    .unwrap();
    let plan = project::plan(&args.command).unwrap();
    assert_eq!(plan.method, reqwest::Method::DELETE);
    assert_eq!(plan.path, "/api/project/todos/a%2Fb/comments/12");
    assert_eq!(plan.query, [("expected_revision".into(), "2".into())]);
    assert!(Args::try_parse_from(["ctl", "todos", "comments", "delete", "t", "12"]).is_err());
    let args = Args::try_parse_from([
        "ctl",
        "todos",
        "comments",
        "delete",
        "t",
        "0",
        "--expected-revision",
        "1",
    ])
    .unwrap();
    assert!(project::plan(&args.command).is_err());
}
