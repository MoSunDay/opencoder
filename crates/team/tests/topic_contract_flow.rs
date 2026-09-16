//! Topic-progression contract, end to end: the captain (and only the
//! captain) plans — opening a topic, calling participants, chasing answers
//! for in-round misalignment, then closing and moving to the next topic.
//! Members always answer in act mode (free text, never a plan decision).
//! One scripted 19-prompt conversation drives TWO topics:
//!
//!   topic 1《回调超时与降级》: turn 1 sub 0 three answers disagree →
//!     captain calls out backend + risk for a clarification sub-turn (sre
//!     is NOT pulled back in) → aligned → turn 2 (participants shrink to
//!     backend + risk) → closing completes
//!   topic 2《重试与幂等》: one aligned round → closing completes
//!
//! The two `final_summary`es concatenate into the deliverable conclusion.

mod common;

use std::sync::Arc;

use common::*;
use opencoder_store::{NodeRecord, Store, TEAM_RUN_FINISHED};
use opencoder_team::{layout, ok, CancelToken, MockDispatcher, RESULT_ALIGNMENT};
use serde_json::json;

const TEAM: &str = "pay-alignment";
const REQ_1: &str = "确定支付回调的超时阈值与降级策略";
const REQ_2: &str = "确定回调失败后的重试与幂等方案";

// ── decision-JSON builders (same shapes as runtime_flow.rs) ───────────────

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

fn ids<'a>(members: impl IntoIterator<Item = &'a NodeRecord>) -> Vec<String> {
    members.into_iter().map(|m| m.id.clone()).collect()
}

// ── fixtures ──────────────────────────────────────────────────────────────

/// The pay-alignment team: real registered nodes carrying a capability
/// snapshot (what the captain's plan prompt sees).
async fn pay_team(fx: &Fixture) -> (NodeRecord, NodeRecord, NodeRecord, NodeRecord) {
    let captain = register(&fx.store, "arch-captain").await;
    let backend = register(&fx.store, "pay-backend").await;
    let sre = register(&fx.store, "sre").await;
    let risk = register(&fx.store, "risk-ctl").await;
    let caps = |name: &str| match name {
        "arch-captain" => vec!["架构决策".to_string()],
        "pay-backend" => vec!["支付网关".to_string(), "回调链路".to_string()],
        "sre" => vec!["容量与稳定性".to_string()],
        _ => vec!["风控规则".to_string(), "降级开关".to_string()],
    };
    let team = opencoder_team::types::TeamMeta {
        name: TEAM.to_string(),
        captain: opencoder_team::types::MemberRef {
            node_id: captain.id.clone(),
            name: captain.name.clone(),
        },
        members: [&backend, &sre, &risk]
            .iter()
            .map(|n| opencoder_team::types::TeamMember {
                node_id: n.id.clone(),
                name: n.name.clone(),
                capabilities: caps(n.name.as_str()),
                profiled_at: Some(1_000),
            })
            .collect(),
        created_at: 1_000,
        updated_at: 1_000,
    };
    opencoder_team::fs_store::create_team(fx.root(), &team).unwrap();
    (captain, backend, sre, risk)
}

async fn start(fx: &Fixture, title: &str, requirement: &str) -> String {
    opencoder_team::start_topic(fx.store.clone(), &fx.cfg, TEAM, title, requirement)
        .await
        .unwrap()
        .topic_id
}

async fn run(
    fx: &Fixture,
    mock: Arc<MockDispatcher>,
    topic_id: &str,
) -> opencoder_team::types::TopicMeta {
    opencoder_team::run_topic(
        fx.store.clone(),
        mock,
        &fx.cfg,
        TEAM,
        topic_id,
        CancelToken::new(),
    )
    .await
    .unwrap()
}

/// The full 19-prompt script: 10 captain decisions + 9 member act-mode
/// answers, per-node FIFO.
fn script(
    fx: &Fixture,
    captain: &NodeRecord,
    backend: &NodeRecord,
    sre: &NodeRecord,
    risk: &NodeRecord,
) -> MockDispatcher {
    let all = ids(&[backend.clone(), sre.clone(), risk.clone()]);
    MockDispatcher::with_store(fx.store.clone())
        .reply(
            &captain.id,
            vec![
                ok(plan("回调超时阈值设多少", &all)),
                ok(summary(
                    "超时阈值与降级口径未对齐",
                    false,
                    &[
                        (&backend.id, "渠道真实 P99 是多少？请给出数据依据"),
                        (&risk.id, "降级放行的开关归属谁"),
                    ],
                )),
                ok(summary("超时上限取 3.5s，超时降级放行", true, &[])),
                ok(closing_continue("降级放行的具体形态")),
                ok(plan(
                    "降级后如何放行与记账",
                    &[backend.id.clone(), risk.id.clone()],
                )),
                ok(summary("降级放行与记账口径一致", true, &[])),
                ok(closing_complete(
                    "回调超时按渠道 P99 2.4s 的 1.5 倍取 3.5s，超时降级放行并标记事后回捞",
                )),
                ok(plan(
                    "失败后的重试与幂等",
                    &[backend.id.clone(), sre.id.clone()],
                )),
                ok(summary("重试 3 次加幂等键的方案一致", true, &[])),
                ok(closing_complete(
                    "失败重试 3 次（指数退避），以 request_no 作幂等键去重",
                )),
            ],
        )
        .reply(
            &backend.id,
            vec![
                ok("我建议同步等待 5s 超时"),
                ok("对齐：渠道实测 P99 2.4s，按 1.5 倍取 3.5s"),
                ok("降级放行：先落库标记 degraded，事后补偿"),
                ok("重试 3 次指数退避，幂等键用 request_no"),
            ],
        )
        .reply(
            &sre.id,
            vec![
                ok("渠道 P99 2.4s，5s 太长，建议 3.5s"),
                ok("重试 3 次可接受，必须幂等防重"),
            ],
        )
        .reply(
            &risk.id,
            vec![
                ok("超时必须降级放行，不能阻塞主链路"),
                ok("对齐：降级开关由风控平台持有，回调侧只执行"),
                ok("放行必须携带风控降级标记"),
            ],
        )
}

// ── the contract ──────────────────────────────────────────────────────────

#[tokio::test]
async fn captain_progresses_two_topics_and_members_stay_in_act_mode() {
    let fx = fixture(4, 3).await;
    let (captain, backend, sre, risk) = pay_team(&fx).await;

    // Topic 1 opens with all three members; topic 2 is a NEW topic the
    // captain opens after topic 1 completes.
    let topic1 = start(&fx, "回调超时与降级", REQ_1).await;
    let topic2 = start(&fx, "重试与幂等", REQ_2).await;
    let mock = Arc::new(script(&fx, &captain, &backend, &sre, &risk));

    let meta1 = run(&fx, mock.clone(), &topic1).await;
    let meta2 = run(&fx, mock.clone(), &topic2).await;

    // ── both topics complete with their own final_summary ────────────────
    assert_eq!(meta1.status, "finished");
    assert_eq!(meta1.finish_reason.as_deref(), Some("complete"));
    assert_eq!(meta2.status, "finished");
    assert_eq!(meta2.finish_reason.as_deref(), Some("complete"));
    let text1 = meta1.final_summary.as_deref().unwrap();
    let text2 = meta2.final_summary.as_deref().unwrap();
    assert!(text1.contains("2.4s") && text1.contains("放行"));
    assert!(text2.contains("重试 3 次") && text2.contains("幂等键"));
    // The deliverable conclusion concatenates both topics' summaries.
    let conclusion = format!("{text1}{text2}");
    for marker in ["2.4s", "放行", "重试 3 次", "幂等键"] {
        assert!(conclusion.contains(marker), "conclusion misses {marker}");
    }

    // ── topic 1: two turns (2 + 1 sub-turns); topic 2: one aligned round ─
    assert_eq!(meta1.turns.len(), 2);
    assert_eq!(meta1.turns[0].turn, 1);
    assert!(meta1.turns[0].aligned);
    assert_eq!(meta1.turns[0].sub_turns, 2, "turn 1 needed a follow-up");
    assert_eq!(meta1.turns[0].participants, ids([&backend, &sre, &risk]));
    assert_eq!(meta1.turns[1].turn, 2);
    assert_eq!(meta1.turns[1].sub_turns, 1);
    assert_eq!(meta1.turns[1].participants, ids([&backend, &risk]));
    assert_eq!(meta2.turns.len(), 1);
    assert_eq!(meta2.turns[0].sub_turns, 1);
    assert!(meta2.turns[0].aligned);
    assert_eq!(meta2.turns[0].participants, ids([&backend, &sre]));

    // ── follow-up sub-turn: only the called-out members answer ───────────
    let root = fx.root();
    let follow_up = layout::sub_dir(root, TEAM, &topic1, 1, 1).unwrap();
    let mut answered = layout::list_valid_members(&follow_up).unwrap();
    let mut expected = ids([&backend, &risk]);
    answered.sort();
    expected.sort();
    assert_eq!(answered, expected, "sre must not be re-asked");
    for node in [backend.id.as_str(), risk.id.as_str()] {
        let rec = opencoder_team::read_result(root, TEAM, &topic1, 1, 1, node)
            .unwrap()
            .unwrap();
        assert_eq!(rec.kind, RESULT_ALIGNMENT);
        assert!(rec.ok);
    }
    assert!(
        opencoder_team::read_result(root, TEAM, &topic1, 1, 1, &sre.id)
            .unwrap()
            .is_none()
    );
    assert!(
        !layout::sub_dir(root, TEAM, &topic1, 1, 2).unwrap().exists(),
        "alignment converged, no phantom third sub-turn"
    );
    // Sub 0 kept all three answers; each sub-turn carries its own summary.
    let sub0 = opencoder_team::read_summary(root, TEAM, &topic1, 1, 0)
        .unwrap()
        .unwrap();
    assert!(!sub0.aligned);
    assert_eq!(sub0.ambiguities.len(), 2);
    assert!(
        opencoder_team::read_summary(root, TEAM, &topic1, 1, 1)
            .unwrap()
            .unwrap()
            .aligned
    );
    assert!(
        opencoder_team::read_summary(root, TEAM, &topic1, 2, 0)
            .unwrap()
            .unwrap()
            .aligned
    );

    // ── prompt contracts ─────────────────────────────────────────────────
    // Members act only; the captain's 10 calls are all decision prompts.
    let captain_calls = mock.calls_for(&captain.id);
    assert_eq!(captain_calls.len(), 10, "10 captain decisions");
    assert!(
        captain_calls
            .iter()
            .all(|c| c.prompt.contains("只输出 JSON")),
        "every captain call is a JSON decision prompt"
    );
    // Members are never asked for JSON — free-text act answers only.
    for node in [&backend, &sre, &risk] {
        for call in mock.calls_for(&node.id) {
            assert!(
                !call.prompt.contains("只输出 JSON"),
                "member prompt must not demand JSON"
            );
        }
    }
    // The follow-up prompts carry the captain's clarification questions.
    let backend_calls = mock.calls_for(&backend.id);
    assert_eq!(backend_calls.len(), 4);
    assert!(backend_calls[1].prompt.contains("需要你澄清"));
    assert!(backend_calls[1].prompt.contains("渠道真实 P99"));
    let risk_calls = mock.calls_for(&risk.id);
    assert_eq!(risk_calls.len(), 3);
    assert!(risk_calls[1].prompt.contains("需要你澄清"));
    assert!(risk_calls[1].prompt.contains("降级放行的开关归属谁"));
    // sre answered exactly once per topic, under the right topic id.
    let sre_calls = mock.calls_for(&sre.id);
    assert_eq!(sre_calls.len(), 2);
    assert_eq!(sre_calls[0].topic.as_deref(), Some(topic1.as_str()));
    assert_eq!(sre_calls[1].topic.as_deref(), Some(topic2.as_str()));
    assert_eq!(mock.call_count(), 19, "10 decisions + 9 act answers");

    // ── ledger: deduped (topic, node) rows, all flipped to finished ───────
    let rows1 = fx.store.list_team_topic_runs(&topic1).await.unwrap();
    assert_eq!(rows1.len(), 4, "captain + 3 members");
    assert!(rows1.iter().all(|r| r.status == TEAM_RUN_FINISHED));
    let rows2 = fx.store.list_team_topic_runs(&topic2).await.unwrap();
    assert_eq!(rows2.len(), 3, "captain + backend + sre");
    assert!(rows2.iter().all(|r| r.status == TEAM_RUN_FINISHED));

    // ── re-running a completed topic dispatches nothing ──────────────────
    let idle = Arc::new(MockDispatcher::with_store(fx.store.clone()));
    let again = run(&fx, idle.clone(), &topic1).await;
    assert_eq!(again.finish_reason.as_deref(), Some("complete"));
    assert_eq!(idle.call_count(), 0);
}
