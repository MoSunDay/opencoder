//! Transient skill full-body delivery — integration contract tests.
//!
//! An activated skill (`> Source:`-prefixed body, as `body_with_source`
//! stores) ships its path + merged body as a TRANSIENT per-call payload
//! message (`[skill loaded] <path>` marker block), appended after the
//! transcript by `runner/llm_call.rs` for EVERY LLM round of the run that
//! armed the skill — eliminating the "model must `read` SKILL.md" tool-call
//! round-trip. The message is NEVER recorded to the transcript or the
//! store, so run end (`skill_lifecycle::clear_on_run_end`) stops the
//! submission entirely: subsequent runs start skill-less. Bodies over ~20K
//! tokens ship as a whole-line prefix plus an `[INCOMPLETE SKILL]` notice
//! whose `offset=` aligns with the read tool (chain-continuation); the
//! tail reminder stays a fallback pointer for the degenerate empty-body
//! case only.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, Store};

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// A tool-call turn so one run spans several LLM rounds (each round must
/// re-derive the transient body while the skill stays armed).
fn bash_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![CompletedToolCall {
            id: "tu1".into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": "true" }),
        }],
        usage: Some(Usage::default()),
    }
}

/// Skill body exactly as the TUI `$` picker / `skill_resolve` store it.
fn sourced_body(path: &str, body: &str) -> String {
    format!("> Source: {path}\n\n{body}")
}

/// Store-less session on a temp working dir. The TempDir pins the cwd for
/// the session's life.
fn session_on(
    id: &str,
    agent: &str,
    client: Arc<MockChatClient>,
) -> (SessionState, tempfile::TempDir) {
    let workdir = tempfile::tempdir().unwrap();
    let session = SessionState::new(
        id,
        resolve_agent(agent).expect("builtin agent"),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        client,
        workdir.path().to_path_buf(),
    );
    (session, workdir)
}

/// System-message content of a request ("" when absent).
fn system_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

/// Content of the LAST payload message: the slot the transient skill
/// context occupies (nothing may ride after it).
fn last_message_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .last()
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

/// Whether any user-role payload message contains `needle`.
fn any_user_contains(req: &opencoder_llm::ChatRequest, needle: &str) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains(needle))
    })
}

/// Non-overlapping occurrence count of `needle` across all user-role
/// payload messages.
fn count_user_occurrences(req: &opencoder_llm::ChatRequest, needle: &str) -> usize {
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(|c| c.match_indices(needle).count())
        .sum()
}

/// (a) The armed body ships EXACTLY ONCE for the whole activation: the
/// FIRST LLM round's payload carries it as the TRAILING message (marker
/// line + body, system prompt clean, fallback pointer suppressed), every
/// LATER round of the same run carries NEITHER body NOR marker; (b) the
/// message never lands in the in-memory transcript nor in the store; (c)
/// once the run ends and the one-shot skill clears, the following plain
/// run submits neither body nor `[active skill]` tail.
#[tokio::test]
async fn armed_body_ships_once_then_stops_and_is_never_persisted() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    // Run 1 spans TWO LLM rounds (tool call, then Done); run 2 is plain.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("use the tool")])
            .push_script(vec![done_turn("did it")])
            .push_script(vec![done_turn("plain work")]),
    );
    let (mut s, _wd) = session_on("inj-transient", "act", mock.clone());
    s.store = Some(store.clone());
    let path = "/skills/review/SKILL.md";
    let marker = format!("[skill loaded] {path}");
    s.set_skill(Some(sourced_body(path, "REV-STEP-1\nREV-STEP-2")));

    run(&mut s, "do the thing".into(), |_| {}).await.unwrap();

    let run1 = mock.requests();
    assert_eq!(run1.len(), 2, "tool round + done round");

    // Round 1 (delivery round): the body rides exactly once, trailing.
    let req = &run1[0];
    assert_eq!(
        count_user_occurrences(req, "REV-STEP-1"),
        1,
        "delivery round: body rides exactly once"
    );
    assert!(
        any_user_contains(req, &marker),
        "delivery round: marker line names the source path"
    );
    assert_eq!(
        count_user_occurrences(req, &marker),
        1,
        "delivery round: one sorted marker block"
    );
    let last = last_message_content(req);
    assert!(
        last.starts_with(&marker),
        "delivery round: body is the TRAILING payload message: {last:.120}"
    );
    assert!(
        !system_content(req).contains("REV-STEP"),
        "delivery round: system prompt stays clean"
    );
    assert!(
        !any_user_contains(req, "[active skill]"),
        "delivery round: fallback pointer suppressed while the body ships"
    );

    // Round 2..N of the same run: NO skill body anywhere in the payload.
    let req = &run1[1];
    assert_eq!(
        count_user_occurrences(req, "REV-STEP"),
        0,
        "round 2 carries no skill body — one-shot delivery is spent"
    );
    assert!(
        !any_user_contains(req, &marker),
        "round 2 carries no [skill loaded] marker"
    );
    assert!(
        !system_content(req).contains("REV-STEP"),
        "round 2: system prompt stays clean"
    );
    assert!(
        !any_user_contains(req, "[active skill]"),
        "round 2: fallback pointer still suppressed (body was delivered)"
    );

    // (b) Zero persistence: not in the transcript, not in the store.
    assert!(
        !s.messages.iter().any(|m| m.text().contains(&marker)),
        "no [skill loaded] message in session.messages"
    );
    let persisted = store.load_messages(&s.id).await.unwrap();
    assert!(
        !persisted.iter().any(|m| m.text().contains(&marker)),
        "no [skill loaded] message in store.load_messages"
    );

    // (c) The follow-up plain run: no body, no tail.
    run(&mut s, "plain follow up".into(), |_| {}).await.unwrap();
    let run2 = &mock.requests()[2];
    assert_eq!(
        count_user_occurrences(run2, "REV-STEP"),
        0,
        "post-clear run carries no body"
    );
    assert!(
        !any_user_contains(run2, &marker) && !any_user_contains(run2, "[active skill]"),
        "post-clear run carries neither marker nor tail"
    );
    assert!(s.skill_prompt_cloned().is_none(), "one-shot clear ran");
}

/// (e) Compound prompt (`$A $B`): ONE trailing message on the delivery
/// round with a single sorted marker block, the merged body keeping B's
/// inner `> Source:` annotation, each body shipping exactly once — and
/// the second round of the run carries none of it.
#[tokio::test]
async fn compound_body_keeps_inner_annotation_and_one_sorted_marker_block() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("work")])
            .push_script(vec![done_turn("done")]),
    );
    let (mut s, _wd) = session_on("inj-compound", "act", mock.clone());
    s.set_skill(Some(format!(
        "{}\n\n{}",
        sourced_body("/skills/alpha/SKILL.md", "A-BODY"),
        sourced_body("/skills/beta/SKILL.md", "B-BODY")
    )));

    run(&mut s, "compound work".into(), |_| {}).await.unwrap();

    // Delivery round: one sorted block + merged body with inner annotation.
    let req = &mock.requests()[0];
    assert_eq!(
        last_message_content(req),
        "[skill loaded] /skills/alpha/SKILL.md\n\
         [skill loaded] /skills/beta/SKILL.md\n\n\
         A-BODY\n\n> Source: /skills/beta/SKILL.md\n\nB-BODY",
        "delivery round: one sorted block + merged body with inner annotation"
    );
    assert_eq!(
        count_user_occurrences(req, "[skill loaded]"),
        2,
        "delivery round: exactly the two marker lines"
    );
    assert_eq!(count_user_occurrences(req, "A-BODY"), 1);
    assert_eq!(count_user_occurrences(req, "B-BODY"), 1);

    // Round 2 of the same run: marker block and bodies fully absent.
    let req = &mock.requests()[1];
    assert_eq!(
        count_user_occurrences(req, "[skill loaded]"),
        0,
        "round 2 carries no marker block"
    );
    assert_eq!(
        count_user_occurrences(req, "-BODY"),
        0,
        "round 2 carries no body"
    );
}

/// (f) Exclusions: subagents (`explore`) and the todos scheduler
/// (`workflow`, itself Primary-mode) — and skill-less sessions — never get
/// a body message in payload or transcript.
#[tokio::test]
async fn excluded_agents_and_skill_less_sessions_get_no_body() {
    for agent in ["explore", "workflow"] {
        let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
        let (mut s, _wd) = session_on(&format!("inj-excl-{agent}"), agent, mock.clone());
        s.set_skill(Some(sourced_body("/skills/x/SKILL.md", "X-BODY")));

        run(&mut s, "scoped".into(), |_| {}).await.unwrap();

        assert!(
            !s.messages
                .iter()
                .any(|m| m.text().contains("[skill loaded]")),
            "{agent}: no transcript injection"
        );
        assert!(
            !any_user_contains(&mock.requests()[0], "[skill loaded]"),
            "{agent}: no payload injection"
        );
    }

    // Skill-less Primary session: nothing to deliver.
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _wd) = session_on("inj-skillless", "act", mock.clone());
    run(&mut s, "plain work".into(), |_| {}).await.unwrap();
    assert!(
        !any_user_contains(&mock.requests()[0], "[skill loaded]"),
        "skill-less payload carries no marker"
    );
    assert!(!any_user_contains(&mock.requests()[0], "[active skill]"));
}

/// Degenerate armed skill with an EMPTY parsed body (frontmatter-only
/// file): no body message is derived or sent, and the transient tail keeps
/// its `[active skill]` fallback pointer to the source path — the only
/// trace of the skill.
#[tokio::test]
async fn empty_body_skill_ships_no_body_but_keeps_tail_pointer() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _wd) = session_on("inj-empty", "act", mock.clone());
    s.set_skill(Some(sourced_body("/skills/e/SKILL.md", "")));

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let req = &mock.requests()[0];
    assert!(
        !any_user_contains(req, "[skill loaded] /skills/e/SKILL.md"),
        "no marker-only body message (tail's generic marker mention excluded: {})",
        last_message_content(req)
    );
    let last = last_message_content(req);
    assert!(
        last.contains("[active skill]") && last.contains("/skills/e/SKILL.md"),
        "fallback pointer names the source path: {last}"
    );
    assert!(
        !s.messages
            .iter()
            .any(|m| m.text().contains("[skill loaded]")),
        "nothing persisted"
    );
}

/// End-to-end BOM frontmatter: a SKILL.md saved as UTF-8 with BOM (plus
/// blank lines before the fence) parses its frontmatter, and only the
/// post-fence body ships in the transient message — the
/// `name:`/`description:` comment block never reaches the payload.
#[tokio::test]
async fn bom_frontmatter_end_to_end_injects_only_body() {
    let root = tempfile::tempdir().unwrap();
    let skill_md = root.path().join("bom-skill").join("SKILL.md");
    std::fs::create_dir_all(skill_md.parent().unwrap()).unwrap();
    std::fs::write(
        &skill_md,
        "\u{FEFF}\n\n---\nname: bom-skill\ndescription: saved with BOM\n---\nBOM-BODY-ONLY\n",
    )
    .unwrap();

    let skill = opencoder_core::discover_in(root.path())
        .into_iter()
        .next()
        .expect("skill discovered despite BOM");
    assert_eq!(
        skill.body, "BOM-BODY-ONLY",
        "frontmatter stripped, body only"
    );

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _wd) = session_on("inj-bom", "act", mock.clone());
    s.set_skill(Some(opencoder_core::body_with_source(&skill)));

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let path = skill.source.display().to_string();
    let last = last_message_content(&mock.requests()[0]);
    assert!(
        last.starts_with(&format!("[skill loaded] {path}\n\nBOM-BODY-ONLY")),
        "body ships under the marker naming the real source file: {last:.200}"
    );
    assert!(
        !any_user_contains(&mock.requests()[0], "name: bom-skill"),
        "no frontmatter leak"
    );
    assert!(
        !any_user_contains(&mock.requests()[0], "\n---\n"),
        "no fence leak"
    );
}
