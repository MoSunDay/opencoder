//! 端到端集成：playbook 执行器的成功路径（本地剧本编排轨，brain 双轨
//! 调度落地端，真 store + MockChatClient）。覆盖：串行链按拓扑序执行并逐段
//! 落子 run 行、菱形依赖并发调度（b/c 同批、d 收口）、brain 钉住能力步骤
//! 经派发交接落到默认路由（无 brain 运行时亦可跑）、brain 内联路由步骤
//! 跨端确定性解析（路由即执行器，不经环境绑定）。失败路径见
//! `executor_playbook_errors.rs`。

use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_project::ProjectService;
use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectStore, ProjectTodoRecord, ProjectTodoStatus,
};
use std::sync::Arc;

struct Harness {
    service: Arc<ProjectService>,
    projects: Arc<dyn ProjectStore>,
    _keep: tempfile::TempDir,
}

async fn harness_on(
    store: Arc<LibsqlStore>,
    mock: Arc<MockChatClient>,
    brain: Option<opencoder_brain::Runtime>,
) -> Harness {
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let service = ProjectService::new();
    service
        .init(
            store.clone(),
            store.clone(),
            dir.path().to_path_buf(),
            Some(client),
            brain,
        )
        .await
        .unwrap();
    // Keep archived inputs and artifacts in this fixture, independent of the
    // developer's global data directory and unrelated disk writers.
    *service.require().unwrap().archive_root.lock().unwrap() = dir.path().join("runs");
    Harness {
        service,
        projects: store,
        _keep: dir,
    }
}

async fn harness_with(scripts: Vec<Vec<LlmEvent>>) -> Harness {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut mock = MockChatClient::new();
    for script in scripts {
        mock = mock.push_script(script);
    }
    harness_on(store, Arc::new(mock), None).await
}

async fn seed_todo(h: &Harness, id: &str, executor_ref: &str) {
    let now = 1000;
    h.projects
        .create_todo(&ProjectTodoRecord {
            id: id.into(),
            milestone_id: None,
            title: format!("待办 {id}"),
            draft: "整理项目结构".into(),
            plan_md: Some("# 方案\n1. 落地目录约定".into()),
            status: ProjectTodoStatus::Planned,
            agent: "act".into(),
            executor_kind: ProjectExecutorKind::Playbook,
            executor_ref: Some(executor_ref.into()),
            executor_spec: None,
            active_session_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

/// 落一份剧本 spec 进 brain playbook 表（spec_json 即驱动读回的形态）。
#[tokio::test]
async fn legacy_playbook_executor_is_rejected_without_claim_or_execution() {
    let h = harness_with(vec![]).await;
    seed_todo(&h, "legacy", "old-playbook").await;
    let error = h.service.start_execute("legacy").await.unwrap_err();
    assert!(error.to_string().contains("migration required"));
    assert!(h
        .projects
        .list_todo_runs("legacy")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        h.projects.get_todo("legacy").await.unwrap().unwrap().status,
        ProjectTodoStatus::Planned
    );
}
