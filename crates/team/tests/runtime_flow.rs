//! End-to-end topic flows over a tempdir "NFS" + real LibsqlStore, driven by
//! the scripted MockDispatcher. Every assertion reads final state from disk
//! or the store — the same thing a resume or the web layer would see.

mod common;

use std::sync::Arc;

use common::*;
use opencoder_store::{Store, TEAM_RUN_FINISHED};
use opencoder_team::{err, fs_store, layout, ok, MockDispatcher};
use serde_json::json;

fn plan(question: &str, participants: &[String]) -> String {
    json!({"question": question, "participants": participants, "rationale": "理由"}).to_string()
}

fn summary(text: &str, aligned: bool, ambiguities: &[(&str, &str)]) -> String {
    json!({
        "summary": text,
        "aligned": aligned,
        "ambiguities": ambiguities
            .iter()
            .map(|(node_id, question)| json!({"node_id": node_id, "question": question}))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

fn closing_complete(final_summary: &str) -> String {
    json!({"complete": true, "next_question": null, "final_summary": final_summary}).to_string()
}

fn closing_continue(next_question: &str) -> String {
    json!({"complete": false, "next_question": next_question, "final_summary": null}).to_string()
}

fn ids(members: &[opencoder_store::NodeRecord]) -> Vec<String> {
    members.iter().map(|m| m.id.clone()).collect()
}

#[tokio::test]
async fn aligned_first_round_completes_with_full_disk_layout() {
    let fx = fixture(3, 2).await;
    let (captain, members) = make_team(&fx, 2).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "布局讨论").await;

    let mock = MockDispatcher::with_store(fx.store.clone())
        .reply(
            &captain.id,
            vec![
                ok(plan("目录怎么组织", &member_ids)),
                ok(summary("全员一致", true, &[])),
                ok(closing_complete("最终结论：按功能分目录")),
            ],
        )
        .reply(&members[0].id, vec![ok("成员0：按功能分")])
        .reply(&members[1].id, vec![ok("成员1：同意")]);
    let meta = run(&fx, Arc::new(mock), &topic_id).await;

    assert_eq!(meta.status, "finished");
    assert_eq!(meta.finish_reason.as_deref(), Some("complete"));
    assert_eq!(
        meta.final_summary.as_deref(),
        Some("最终结论：按功能分目录")
    );
    assert_eq!(meta.turns.len(), 1);
    assert_eq!(meta.turns[0].turn, 1);
    assert!(meta.turns[0].aligned);
    assert_eq!(meta.turns[0].sub_turns, 1);
    assert_eq!(meta.turns[0].participants, member_ids);

    let root = fx.root();
    assert!(layout::plan_file(root, TEAM, &topic_id, 1)
        .unwrap()
        .exists());
    for id in &member_ids {
        assert!(layout::result_file(root, TEAM, &topic_id, 1, 0, id)
            .unwrap()
            .exists());
    }
    assert!(layout::summary_file(root, TEAM, &topic_id, 1, 0)
        .unwrap()
        .exists());
    assert!(!layout::sub_dir(root, TEAM, &topic_id, 1, 1)
        .unwrap()
        .exists());

    let rows = fx.store.list_team_topic_runs(&topic_id).await.unwrap();
    assert_eq!(rows.len(), 3, "captain + both members get ledger rows");
    assert!(rows.iter().all(|r| r.status == TEAM_RUN_FINISHED));

    // Re-running a finished topic is an idempotent no-op (no dispatches).
    let empty = Arc::new(MockDispatcher::with_store(fx.store.clone()));
    let again = run(&fx, empty.clone(), &topic_id).await;
    assert_eq!(again.finish_reason.as_deref(), Some("complete"));
    assert_eq!(empty.call_count(), 0);
}

#[tokio::test]
async fn ambiguity_runs_a_second_alignment_sub_turn() {
    let fx = fixture(3, 3).await;
    let (captain, members) = make_team(&fx, 2).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "技术选型").await;

    let mock = MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok(plan("选 SQLite 还是客户端存储", &member_ids)),
                ok(summary(
                    "成员0仍有歧义",
                    false,
                    &[(&members[0].id, "依赖冲突如何处理")],
                )),
                ok(summary("澄清后对齐", true, &[])),
                ok(closing_complete("结论：libsql")),
            ],
        )
        .reply(
            &members[0].id,
            vec![ok("首选 libsql"), ok("冲突用构建特性隔离")],
        )
        .reply(&members[1].id, vec![ok("同意 libsql")]);
    let meta = run(&fx, Arc::new(mock), &topic_id).await;

    let root = fx.root();
    let m0 = fs_store::read_result(root, TEAM, &topic_id, 1, 1, &members[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(m0.kind, "alignment");
    assert!(m0.answer.contains("冲突"));
    assert!(
        fs_store::read_result(root, TEAM, &topic_id, 1, 1, &members[1].id)
            .unwrap()
            .is_none(),
        "only the ambiguous member is re-asked"
    );
    assert_eq!(meta.turns[0].sub_turns, 2);
    assert_eq!(meta.finish_reason.as_deref(), Some("complete"));
}

#[tokio::test]
async fn persistent_misalignment_finishes_with_max_sub_turns() {
    let fx = fixture(3, 1).await;
    let (captain, members) = make_team(&fx, 2).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "争论话题").await;

    let mock = MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok(plan("争论", &member_ids)),
                ok(summary("成员0有歧义", false, &[(&members[0].id, "再解释")])),
                ok(summary("仍然分歧", false, &[])),
            ],
        )
        .reply(&members[0].id, vec![ok("观点A"), ok("坚持A")])
        .reply(&members[1].id, vec![ok("观点B")]);
    let meta = run(&fx, Arc::new(mock), &topic_id).await;

    assert_eq!(meta.status, "finished");
    assert_eq!(meta.finish_reason.as_deref(), Some("max_sub_turns"));
    assert_eq!(meta.final_summary.as_deref(), Some("仍然分歧"));
    assert!(!meta.turns[0].aligned);
    assert_eq!(meta.turns[0].sub_turns, 2);
}

#[tokio::test]
async fn member_failure_is_tolerated_and_recorded() {
    let fx = fixture(3, 2).await;
    let (captain, members) = make_team(&fx, 2).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "容错").await;

    let mock = MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok(plan("问题", &member_ids)),
                ok(summary("忽略失败成员", true, &[])),
                ok(closing_complete("结论")),
            ],
        )
        .reply(&members[0].id, vec![err("node down")])
        .reply(&members[1].id, vec![ok("正常回答")]);
    let meta = run(&fx, Arc::new(mock), &topic_id).await;

    let rec = fs_store::read_result(fx.root(), TEAM, &topic_id, 1, 0, &members[0].id)
        .unwrap()
        .unwrap();
    assert!(!rec.ok);
    assert!(rec.error.as_deref().unwrap().contains("node down"));
    assert_eq!(meta.finish_reason.as_deref(), Some("complete"));
}

#[tokio::test]
async fn captain_json_correction_reasks_with_feedback() {
    let fx = fixture(3, 2).await;
    let (captain, members) = make_team(&fx, 1).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "纠错").await;

    let mock = MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok("这个问题很复杂，我需要先想想，无法直接给出计划。"),
                ok(plan("重新规划的问题", &member_ids)),
                ok(summary("对齐", true, &[])),
                ok(closing_complete("结论")),
            ],
        )
        .reply(&members[0].id, vec![ok("回答")]);
    let mock = Arc::new(mock);
    let meta = run(&fx, mock.clone(), &topic_id).await;

    let captain_calls = mock.calls_for(&captain.id);
    assert_eq!(captain_calls.len(), 4);
    let correction = &captain_calls[1];
    assert!(
        correction.prompt.contains("无法"),
        "correction carries the parse error"
    );
    assert!(
        correction.prompt.contains("这个问题很复杂"),
        "correction quotes the raw reply"
    );
    assert_eq!(captain_calls[0].topic.as_deref(), Some(topic_id.as_str()));
    assert_eq!(meta.finish_reason.as_deref(), Some("complete"));
}

#[tokio::test]
async fn never_completing_captain_stops_at_max_turns() {
    let fx = fixture(2, 2).await;
    let (captain, members) = make_team(&fx, 1).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "多轮").await;

    let mock = MockDispatcher::new()
        .reply(
            &captain.id,
            vec![
                ok(plan("第一问", &member_ids)),
                ok(summary("第一轮小结", true, &[])),
                ok(closing_continue("第二问")),
                ok(plan("第二问", &member_ids)),
                ok(summary("第二轮小结", true, &[])),
                ok(closing_continue("第三问")),
            ],
        )
        .reply(&members[0].id, vec![ok("答1"), ok("答2")]);
    let meta = run(&fx, Arc::new(mock), &topic_id).await;

    assert_eq!(meta.turns.len(), 2);
    assert_eq!(meta.finish_reason.as_deref(), Some("max_turns"));
    assert_eq!(meta.final_summary.as_deref(), Some("第二轮小结"));
    assert!(layout::plan_file(fx.root(), TEAM, &topic_id, 2)
        .unwrap()
        .exists());
    assert!(!layout::turn_dir(fx.root(), TEAM, &topic_id, 3)
        .unwrap()
        .exists());
}

#[tokio::test]
async fn errored_topic_resumes_from_disk_without_replaying_done_work() {
    let fx = fixture(3, 2).await;
    let (captain, members) = make_team(&fx, 2).await;
    let member_ids = ids(&members);
    let topic_id = start(&fx, "续跑").await;

    // Phase 1: plan + member answers land on disk, then the captain's
    // summary dispatch fails twice (1 retry) -> finish(error).
    let mock = MockDispatcher::with_store(fx.store.clone())
        .reply(
            &captain.id,
            vec![
                ok(plan("问题", &member_ids)),
                err("队长掉线"),
                err("队长仍掉线"),
            ],
        )
        .reply(&members[0].id, vec![ok("答A")])
        .reply(&members[1].id, vec![ok("答B")]);
    let errored = run(&fx, Arc::new(mock), &topic_id).await;
    assert_eq!(errored.status, "finished");
    assert_eq!(errored.finish_reason.as_deref(), Some("error"));
    assert!(
        layout::result_file(fx.root(), TEAM, &topic_id, 1, 0, &members[0].id)
            .unwrap()
            .exists()
    );
    assert!(!layout::summary_file(fx.root(), TEAM, &topic_id, 1, 0)
        .unwrap()
        .exists());
    let rows = fx.store.list_team_topic_runs(&topic_id).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.status == TEAM_RUN_FINISHED));

    // Phase 2: a fresh script continues from the summary decision only.
    let resume = Arc::new(MockDispatcher::new().reply(
        &captain.id,
        vec![
            ok(summary("续跑后对齐", true, &[])),
            ok(closing_complete("续跑结论")),
        ],
    ));
    let meta = run(&fx, resume.clone(), &topic_id).await;
    assert_eq!(meta.finish_reason.as_deref(), Some("complete"));
    assert_eq!(meta.turns.len(), 1);
    assert_eq!(resume.call_count(), 2, "only summary + closing re-asked");
    assert!(
        resume.calls_for(&members[0].id).is_empty(),
        "answered members are not re-dispatched"
    );
    assert!(resume.calls_for(&members[1].id).is_empty());
}
