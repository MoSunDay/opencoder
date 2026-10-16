//! CLI file commands against a real isolated Server router.
use opencoder_cli::{
    cmd::project::{self, files::FilesCmd, ProjectCmd, TodosCmd},
    ctx::Ctx,
};
use serde_json::{json, Value};

#[tokio::test]
async fn cli_upload_download_list_delete_and_handoff_context() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(
        work.join("opencoder.json"),
        json!({"project":{"todo_storage":{"files_dir":root.path().join("files")}}}).to_string(),
    )
    .unwrap();
    let state = opencoder_control::new_state(work, root.path().join("state"), None)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ctx = Ctx {
        server: format!("http://{}", listener.local_addr().unwrap()),
        token: Some("test-files".into()),
        verbose: false,
        http: opencoder_cli::http::client().unwrap(),
    };
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            opencoder_control::build_app(state, Some("test-files".into()), false),
        )
        .await
        .unwrap()
    });
    let todo: Value = ctx
        .http
        .post(format!("{}/api/project/todos", ctx.server))
        .bearer_auth("test-files")
        .json(&json!({"title":"Task","draft":"current handoff"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = todo["id"].as_str().unwrap().to_owned();
    let file = root.path().join("source.bin");
    std::fs::write(&file, b"\0\xfftask bytes").unwrap();
    let command = |command| ProjectCmd::Todos(TodosCmd::Files(command));
    assert_eq!(
        project::run(
            &ctx,
            command(FilesCmd::Upload {
                id: id.clone(),
                file: file.clone(),
                path: Some("logs/file.bin".into())
            })
        )
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        project::run(
            &ctx,
            command(FilesCmd::Upload {
                id: id.clone(),
                file: file.clone(),
                path: Some("logs/file.bin".into())
            })
        )
        .await
        .unwrap(),
        4
    );
    assert_eq!(
        project::run(&ctx, command(FilesCmd::List { id: id.clone() }))
            .await
            .unwrap(),
        0
    );
    let context = project::context::collect(
        &ctx,
        &project::context::Options {
            id: id.clone(),
            execution: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(context["complete"], true);
    assert_eq!(
        context["record"]["files"]["files"][0]["path"],
        "logs/file.bin"
    );
    assert!(context["record"]["files"]["files"][0]["download_path"]
        .as_str()
        .unwrap()
        .contains("/file?path="));
    let output = root.path().join("download.bin");
    assert_eq!(
        project::run(
            &ctx,
            command(FilesCmd::Download {
                id: id.clone(),
                path: "logs/file.bin".into(),
                output: output.clone()
            })
        )
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read(&output).unwrap(),
        std::fs::read(&file).unwrap()
    );
    assert!(project::run(
        &ctx,
        command(FilesCmd::Download {
            id: id.clone(),
            path: "logs/file.bin".into(),
            output: output.clone()
        })
    )
    .await
    .is_err());
    assert_eq!(
        project::run(
            &ctx,
            command(FilesCmd::Delete {
                id: id.clone(),
                path: "logs/file.bin".into()
            })
        )
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        project::run(
            &ctx,
            command(FilesCmd::Download {
                id,
                path: "missing".into(),
                output: root.path().join("missing")
            })
        )
        .await
        .unwrap(),
        4
    );
    assert!(!root.path().join("missing").exists());
    task.abort();
}
