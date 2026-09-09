//! Multi-round team consensus flow against the real control app + WS worker
//! node: two captain rounds with an alignment sub-turn in round one, the
//! closing "continue" verdict feeding `next_question` into the next round's
//! plan prompt, and the full worker-side topic state (turn ledger, per
//! sub-turn results with `answer`/`alignment` kinds, summaries, plans).

use super::*;
use opencoder_llm::ChatRequest;

fn completed(text: String) -> Vec<LlmEvent> {
    // Member sessions capture their transcript from TextDelta frames, so
    // every scripted reply needs the delta plus the terminal Completed.
    vec![
        LlmEvent::TextDelta(text.clone()),
        LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        },
    ]
}

/// Concatenated message content of one chat request (prompt-matching helper).
fn prompt_text(request: &ChatRequest) -> String {
    request
        .messages
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
async fn team_multiround_consensus_runs_alignment_subturn_and_next_round_hint() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let saved = fleet
        .call(
            "POST",
            "/api/teams",
            json!({"name":"consensus-team","captain":"captain","members":[
                {"id":"captain","agent":"act","role":"coordinate"},
                {"id":"arch","agent":"act","role":"architect"},
                {"id":"qa","agent":"act","role":"qa"}
            ]}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");

    // 12 scripted replies in exact consumption order:
    //  1 plan(t1)            2 arch answer        3 qa answer (vague)
    //  4 summary(t1,0)=unaligned                  5 qa alignment follow-up
    //  6 summary(t1,1)=aligned 7 closing(t1)=continue(+next_question)
    //  8 plan(t2, hinted)    9 arch answer       10 qa answer
    // 11 summary(t2,0)=aligned                    12 closing(t2)=complete
    for text in [
        json!({"question":"define the caching strategy","participants":["arch","qa"],"rationale":"need design and quality views"}).to_string(),
        "arch: Redis LRU with a 24h TTL in front of the primary store, keys namespaced by entity.".into(),
        "qa: direction works, but invalidation on deploy is unclear to me.".into(),
        json!({"summary":"caching direction agreed","aligned":false,"ambiguities":[{"node_id":"qa","question":"invalidation on deploy?"}]}).to_string(),
        "qa: embed the release version in the cache key so a deploy invalidates atomically by key rotation.".into(),
        json!({"summary":"invalidation settled via versioned keys","aligned":true,"ambiguities":[]}).to_string(),
        json!({"complete":false,"next_question":"verify cache failure modes under node loss","final_summary":null}).to_string(),
        json!({"question":"verify cache failure modes under node loss","participants":["arch","qa"],"rationale":"verify the agreed design"}).to_string(),
        "arch: on node loss the LRU entries expire via TTL; no stale reads because writes bypass the cache.".into(),
        "qa: acceptance: no stale reads beyond one TTL window and zero cache-layer errors during a node-loss drill.".into(),
        json!({"summary":"failure modes verified with acceptance criteria","aligned":true,"ambiguities":[]}).to_string(),
        json!({"complete":true,"next_question":null,"final_summary":"consensus reached: versioned-key LRU cache with atomic deploy invalidation and verified node-loss failure modes"}).to_string(),
    ] {
        client.queue_script(completed(text));
    }

    let dispatched = fleet
        .call(
            "POST",
            "/api/executions",
            json!({"id":"team-consensus-1","kind":"team","target":"consensus-team","input":{"prompt":"design the caching strategy and verify it"}}),
        )
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");

    // ── execution layer: settled as done with member- sessions on the node.
    let detail = settled(&fleet.nodes[0], "team-consensus-1").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");
    assert!(fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .iter()
        .any(|index| index.id.starts_with("member-")));

    // ── request layer: exactly the 12 scripted decisions/answers, in order.
    let requests = client.requests();
    assert_eq!(requests.len(), 12, "{:?}", requests.len());
    let request_prompt = |i: usize| prompt_text(&requests[i]);
    assert!(
        request_prompt(1).contains("define the caching strategy"),
        "arch answer prompt must carry the t1 question: {}",
        request_prompt(1)
    );
    let alignment = request_prompt(4);
    assert!(
        alignment.contains("需要你针对性澄清"),
        "alignment sub-turn must use the alignment prompt: {alignment}"
    );
    assert!(
        alignment.contains("invalidation on deploy?"),
        "alignment prompt must carry the ambiguity question: {alignment}"
    );
    assert!(
        request_prompt(7).contains("上一轮建议的下一问题：verify cache failure modes under node loss"),
        "t2 plan prompt must carry the closing next_question hint: {}",
        request_prompt(7)
    );

    // ── state layer: worker-side topic metadata (turns are 1-based).
    let topic_dir = fleet
        .root()
        .join("n0/node/team/team-consensus-1/team/consensus-team/team-consensus-1");
    let topic: Value = read_json(&topic_dir.join("team.json"));
    assert_eq!(topic["status"], json!("finished"));
    assert_eq!(topic["finish_reason"], json!("complete"));
    assert_eq!(
        topic["final_summary"],
        json!("consensus reached: versioned-key LRU cache with atomic deploy invalidation and verified node-loss failure modes")
    );
    let turns = topic["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2, "{turns:?}");
    assert_eq!(
        turns[0],
        json!({"turn":1,"question":"define the caching strategy","participants":["arch","qa"],"aligned":true,"sub_turns":2})
    );
    assert_eq!(
        turns[1],
        json!({"turn":2,"question":"verify cache failure modes under node loss","participants":["arch","qa"],"aligned":true,"sub_turns":1})
    );

    // ── artifact layer: turn 1 sub 0 = plain answers for both members.
    assert_eq!(read_json(&topic_dir.join("1/0/arch/result.json"))["kind"], json!("answer"));
    assert_eq!(read_json(&topic_dir.join("1/0/qa/result.json"))["kind"], json!("answer"));
    // Turn 1 sub 0 summary flagged qa as the only ambiguity...
    let summary_1_0 = read_json(&topic_dir.join("1/0/summary.json"));
    assert_eq!(summary_1_0["aligned"], json!(false));
    assert_eq!(summary_1_0["ambiguities"][0]["node_id"], json!("qa"));
    // ...so the alignment sub-turn re-asked ONLY qa.
    assert_eq!(read_json(&topic_dir.join("1/1/qa/result.json"))["kind"], json!("alignment"));
    assert!(!topic_dir.join("1/1/arch").exists());
    assert_eq!(read_json(&topic_dir.join("1/1/summary.json"))["aligned"], json!(true));
    // Turn 2 aligned on the first sub-turn: no follow-up sub-turn directory.
    assert!(!topic_dir.join("2/1").exists());
    // Turn 2 plan mirrors the closing decision's next_question verbatim.
    let plan_2 = read_json(&topic_dir.join("2/plan.json"));
    assert_eq!(
        plan_2["question"],
        json!("verify cache failure modes under node loss")
    );
    assert_eq!(plan_2["participants"], json!(["arch", "qa"]));
    fleet.shutdown().await;
}
