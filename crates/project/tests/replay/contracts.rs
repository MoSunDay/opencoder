use super::*;
use serde_json::{json, Value};

fn parse(raw: &Option<String>) -> Value {
    serde_json::from_str(raw.as_ref().unwrap()).unwrap()
}

#[tokio::test]
async fn retries_pin_input_and_agent_changes_start_new_sessions() {
    let h = harness(vec![
        done("original result"),
        done("second result"),
        done("new agent result"),
    ])
    .await;
    let todo = seed_todo(&h.projects, "replay", None).await;
    h.projects
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                plan_md: Some(Some("original plan".into())),
                status: Some(ProjectTodoStatus::Planned),
                ..Default::default()
            },
            1,
        )
        .await
        .unwrap();
    let request = json!({"action":"execute"});
    let first = h
        .service
        .start_attempt(
            &todo,
            ProjectTodoRunKind::Execute,
            Some("prun-stable"),
            None,
            request.clone(),
        )
        .await
        .unwrap();
    let original = wait_run_done(&h.projects, &first).await;
    assert_eq!(original.status, ProjectTodoRunStatus::Done, "{original:?}");
    let input = parse(&original.input_snapshot);
    let trace = parse(&original.trace_manifest);
    assert!(input["prompt"].as_str().unwrap().contains("original plan"));
    assert_eq!(input["agent"]["name"], "act");
    assert_eq!(trace["complete"], true);
    assert_eq!(trace["model_calls"], 1);
    let root = opencoder_project::trace::root(&h.service.require().unwrap()).join(&first);
    let payload: Value =
        serde_json::from_slice(&std::fs::read(root.join("request-1.json")).unwrap()).unwrap();
    assert!(payload.to_string().contains("original plan"));
    assert!(std::fs::read_to_string(root.join("response-1.jsonl"))
        .unwrap()
        .contains("original result"));
    h.projects
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                draft: Some("changed draft".into()),
                ..Default::default()
            },
            2,
        )
        .await
        .unwrap();
    assert_eq!(
        h.service
            .start_attempt(
                &todo,
                ProjectTodoRunKind::Execute,
                Some(&first),
                None,
                request.clone()
            )
            .await
            .unwrap(),
        first
    );
    assert_eq!(h.mock.call_count(), 1);
    assert!(h
        .service
        .start_attempt(
            &todo,
            ProjectTodoRunKind::Execute,
            Some(&first),
            None,
            json!({"changed":true})
        )
        .await
        .is_err());
    assert_eq!(
        h.projects
            .get_todo_run(&first)
            .await
            .unwrap()
            .unwrap()
            .input_snapshot,
        original.input_snapshot
    );
    let second = h.service.start_execute(&todo).await.unwrap();
    let second = wait_run_done(&h.projects, &second).await;
    assert_eq!(original.session_id, second.session_id);
    assert_eq!(
        parse(&second.trace_manifest)["messages_after"],
        trace["messages_through"]
    );
    h.projects
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                agent: Some("explore".into()),
                ..Default::default()
            },
            3,
        )
        .await
        .unwrap();
    let third = h.service.start_execute(&todo).await.unwrap();
    let third = wait_run_done(&h.projects, &third).await;
    assert_eq!(third.status, ProjectTodoRunStatus::Done);
    assert_ne!(second.session_id, third.session_id);
    let third_input = parse(&third.input_snapshot);
    assert_eq!(third_input["agent"]["name"], "explore");
    assert!(third_input["prompt"]
        .as_str()
        .unwrap()
        .contains("second result"));
    assert_eq!(
        parse(
            &h.projects
                .get_todo_run(&first)
                .await
                .unwrap()
                .unwrap()
                .trace_manifest
        ),
        trace
    );
}

#[tokio::test]
async fn registered_artifacts_keep_original_bytes_and_digest() {
    let tool = vec![LlmEvent::Completed {
        text: "registering".into(),
        tool_calls: vec![CompletedToolCall {
            id: "artifact-call".into(),
            name: "project_artifact".into(),
            input: json!({"path":"report.txt"}),
        }],
        usage: None,
    }];
    let h = harness(vec![done("plan"), tool, done("delivered")]).await;
    std::fs::write(h._dir.path().join("report.txt"), "immutable report").unwrap();
    let todo = seed_todo(&h.projects, "artifact", None).await;
    let plan = h.service.start_plan(&todo).await.unwrap();
    wait_run_done(&h.projects, &plan).await;
    let run = h.service.start_execute(&todo).await.unwrap();
    let run = wait_run_done(&h.projects, &run).await;
    assert_eq!(run.status, ProjectTodoRunStatus::Done, "{run:?}");
    let trace = parse(&run.trace_manifest);
    assert_eq!(trace["artifacts"].as_array().unwrap().len(), 1, "{trace}");
    let artifact = &trace["artifacts"][0];
    std::fs::write(h._dir.path().join("report.txt"), "changed later").unwrap();
    let root = opencoder_project::trace::root(&h.service.require().unwrap()).join(&run.id);
    let bytes = std::fs::read(root.join(artifact["file"].as_str().unwrap())).unwrap();
    assert_eq!(bytes, b"immutable report");
    use sha2::{Digest, Sha256};
    assert_eq!(artifact["sha256"], format!("{:x}", Sha256::digest(bytes)));
}

#[tokio::test]
async fn plan_and_execute_admission_is_atomic_and_versions_are_unique() {
    let h = harness(vec![done("complete")]).await;
    let todo = seed_todo(&h.projects, "atomic", None).await;
    h.projects
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                plan_md: Some(Some("plan".into())),
                ..Default::default()
            },
            1,
        )
        .await
        .unwrap();
    let (plan, execute) = tokio::join!(
        h.service.reserve_attempt(
            &todo,
            ProjectTodoRunKind::Plan,
            "prun-plan",
            None,
            json!({})
        ),
        h.service.reserve_attempt(
            &todo,
            ProjectTodoRunKind::Execute,
            "prun-execute",
            None,
            json!({})
        )
    );
    assert_ne!(plan.is_ok(), execute.is_ok());
    let accepted = plan.or(execute).unwrap();
    assert_eq!(accepted.version, 1);
    assert_eq!(h.projects.list_todo_runs(&todo).await.unwrap().len(), 1);
    h.service.cancel(&accepted.id).await.unwrap();
    let next = h.service.start_plan(&todo).await.unwrap();
    assert_eq!(wait_run_done(&h.projects, &next).await.version, 2);
}

#[tokio::test]
async fn archive_failure_cannot_report_success_or_accept_more_work() {
    let h = harness(vec![done("must not be sent")]).await;
    let todo = seed_todo(&h.projects, "failure", None).await;
    let archive = opencoder_project::trace::root(&h.service.require().unwrap());
    std::fs::create_dir_all(&archive).unwrap();
    // A file where the run directory must be created is a deterministic I/O failure.
    std::fs::write(archive.join("prun-io-failure"), b"blocked").unwrap();
    let id = h
        .service
        .start_attempt(
            &todo,
            ProjectTodoRunKind::Plan,
            Some("prun-io-failure"),
            None,
            json!({}),
        )
        .await
        .unwrap();
    let run = wait_run_done(&h.projects, &id).await;
    assert_eq!(run.status, ProjectTodoRunStatus::Failed);
    assert_eq!(h.mock.call_count(), 0);
    assert!(run.input_snapshot.is_some());
    assert!(h
        .service
        .require()
        .unwrap()
        .persistence_error
        .lock()
        .unwrap()
        .is_some());
    assert!(h.service.start_plan(&todo).await.is_err());
}

#[tokio::test]
async fn child_inputs_outputs_and_links_are_archived_with_the_parent_attempt() {
    let tool = vec![LlmEvent::Completed {
        text: "delegate".into(),
        tool_calls: vec![CompletedToolCall {
            id: "child-call".into(),
            name: "task".into(),
            input: json!({"description":"inspect fixture", "prompt":"child prompt retained", "subagent_type":"explore"}),
        }],
        usage: None,
    }];
    let h = harness(vec![
        tool,
        done("child output retained"),
        done("parent finished"),
    ])
    .await;
    let todo = seed_todo(&h.projects, "child", None).await;
    let run = h.service.start_plan(&todo).await.unwrap();
    let run = wait_run_done(&h.projects, &run).await;
    assert_eq!(run.status, ProjectTodoRunStatus::Done, "{run:?}");
    let trace = parse(&run.trace_manifest);
    assert_eq!(trace["children"].as_array().unwrap().len(), 1, "{trace}");
    let child = &trace["children"][0];
    assert_eq!(child["task_id"], "child-call");
    let session = child["child_session_id"].as_str().unwrap();
    assert!(h.store.get_session(session).await.unwrap().is_some());
    let messages = h.store.load_messages(session).await.unwrap();
    assert!(messages
        .iter()
        .any(|message| message.text().contains("child prompt retained")));
    assert!(messages
        .iter()
        .any(|message| message.text().contains("child output retained")));
    let archive = opencoder_project::trace::root(&h.service.require().unwrap()).join(&run.id);
    assert!(std::fs::read_to_string(archive.join("request-2.json"))
        .unwrap()
        .contains("child prompt retained"));
    assert!(std::fs::read_to_string(archive.join("response-2.jsonl"))
        .unwrap()
        .contains("child output retained"));
    assert_eq!(trace["model_calls"], 3);
}
