#![cfg(unix)]
#[path = "harness/fixtures.rs"]
mod fixtures;

use opencoder_session::{resume, run};
use opencoder_store::SessionMeta;
use serde_json::Value;

#[tokio::test]
async fn codex_operator_defaults_include_private_skills_on_start_resume_and_fork() {
    let root = tempfile::tempdir().unwrap();
    let resources = tempfile::tempdir().unwrap();
    fixtures::card(resources.path());
    let (mut initial, store) = fixtures::session(root.path()).await;
    initial.config.agent.agents_dir = Some(resources.path().into());
    store
        .create_session(&SessionMeta {
            id: initial.id.clone(),
            kind: Some("operator".into()),
            agent: Some("custom".into()),
            title: Some("Operator fixture".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .set_harness_runtime(&initial.id, &initial.harness)
        .await
        .unwrap();
    let skills = root.path().join("home/.opencoder/skills");
    std::fs::create_dir_all(&skills).unwrap();
    opencoder_core::skill::with_execution(Some(skills.clone()), async {
        let mut session = resume(
            store.clone(),
            &initial.id,
            initial.config.clone(),
            initial.client.clone(),
            root.path().into(),
        )
        .await
        .unwrap();
        run(&mut session, "read resources".into(), |_| {})
            .await
            .unwrap();
        let private = session.skill_roots[0].clone();
        assert!(private.join("inspect/SKILL.md").is_file());
        let mut restored = resume(
            store.clone(),
            &initial.id,
            initial.config.clone(),
            initial.client.clone(),
            root.path().into(),
        )
        .await
        .unwrap();
        run(&mut restored, "follow up".into(), |_| {})
            .await
            .unwrap();
        let fork = opencoder_session::fork::fork_session(store.as_ref(), &initial.id)
            .await
            .unwrap();
        let mut forked = resume(
            store.clone(),
            &fork,
            initial.config.clone(),
            initial.client.clone(),
            root.path().into(),
        )
        .await
        .unwrap();
        run(&mut forked, "fork task".into(), |_| {}).await.unwrap();
        let records: Vec<Value> = std::fs::read_to_string(root.path().join("capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        for (record, task) in records
            .iter()
            .zip(["read resources", "follow up", "fork task"])
        {
            let prompt = record["prompt"].as_str().unwrap();
            assert_eq!(prompt.matches("## Operator 默认规则").count(), 1);
            assert!(prompt.contains("每次调用聚焦一个可验证的结果"));
            assert!(prompt.contains(private.to_str().unwrap()));
            assert!(prompt.contains(skills.to_str().unwrap()));
            assert!(prompt.ends_with(task));
        }
        assert!(records[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("SOUL_FIXTURE"));
        assert_eq!(records[1]["args"][1], "resume");
        assert_eq!(records[2]["args"][1], "fork");
    })
    .await;
}
