//! Persistent skill full-body injection — integration contract tests.
//!
//! An activated skill (`> Source:`-prefixed body, as `body_with_source`
//! stores) gets its path + body injected ONCE into the PERSISTENT transcript
//! as a `synthetic=true` user message (`[skill loaded] <path>` marker) before
//! the first LLM round that follows activation — eliminating the "model must
//! `read` SKILL.md" tool-call round-trip. Bodies over ~20K tokens inject as a
//! whole-line prefix plus an `[INCOMPLETE SKILL]` notice whose `offset=`
//! aligns with the read tool (chain-continuation). The transient tail
//! reminder remains the fallback pointer only.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, Store};

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Skill body exactly as the TUI `$` picker / `skill_resolve` store it.
fn sourced_body(path: &str, body: &str) -> String {
    format!("> Source: {path}\n\n{body}")
}

/// Store-less session on a temp working dir. The TempDir pins the cwd for
/// the session's life.
fn session_on(id: &str, agent: &str, client: Arc<MockChatClient>) -> (SessionState, tempfile::TempDir) {
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

/// Content of the LAST user-role message — where the transient tail
/// reminder rides.
fn last_user_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
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

/// The persistent `[skill loaded] <path>` message in the in-memory
/// transcript, if injected.
fn injected<'a>(s: &'a SessionState, path: &str) -> Option<&'a Message> {
    let marker = format!("[skill loaded] {path}\n");
    s.messages
        .iter()
        .find(|m| m.synthetic && m.role == Role::User && m.text().starts_with(&marker))
}

/// Total `[skill loaded]` messages in the transcript (any path).
fn injected_count(s: &SessionState) -> usize {
    s.messages
        .iter()
        .filter(|m| m.synthetic && m.role == Role::User && m.text().starts_with("[skill loaded] "))
        .count()
}

/// 1. Small skill: first turn's payload carries the full body as a `[skill
/// loaded]` user message; the message is synthetic, lives in the persistent
/// transcript AND the store; system prompt and transient tail stay clean.
#[tokio::test]
async fn small_skill_body_rides_payload_and_persists() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("one")])
            .push_script(vec![done_turn("two")]),
    );
    let (mut s, _wd) = session_on("inj-small", "act", mock.clone());
    s.store = Some(store.clone());
    let path = "/skills/review/SKILL.md";
    s.set_skill(Some(sourced_body(path, "REV-STEP-1\nREV-STEP-2")));

    run(&mut s, "do the thing".into(), |_| {}).await.unwrap();

    let req = &mock.requests()[0];
    assert!(
        any_user_contains(req, &format!("[skill loaded] {path}")),
        "marker message rides the payload"
    );
    assert!(any_user_contains(req, "REV-STEP-1\nREV-STEP-2"), "full body");
    assert!(!system_content(req).contains("REV-STEP"), "system clean");
    assert!(
        !last_user_content(req).contains("REV-STEP"),
        "tail reminder carries no body — it is only the fallback pointer"
    );
    assert!(last_user_content(req).contains("[active skill]"));

    let inj = injected(&s, path).expect("marker message recorded in transcript");
    assert!(inj.synthetic, "synthetic flag set");
    assert_eq!(
        inj.text(),
        format!("[skill loaded] {path}\n\nREV-STEP-1\nREV-STEP-2")
    );

    // Durable: survives a store round-trip (resume can replay it).
    let persisted = store.load_messages(&s.id).await.unwrap();
    assert!(
        persisted
            .iter()
            .any(|m| m.synthetic
                && m.role == Role::User
                && m.text().starts_with(&format!("[skill loaded] {path}"))),
        "injection persisted to the store"
    );

    // Idempotent: a second turn must not re-inject.
    run(&mut s, "again".into(), |_| {}).await.unwrap();
    assert_eq!(injected_count(&s), 1, "one-shot per skill path");
    let req2 = &mock.requests()[1];
    assert_eq!(count_user_occurrences(req2, "REV-STEP-1"), 1);
    // System bytes stable across the two turns (prefix-cache contract).
    assert_eq!(system_content(req), system_content(req2));
}

/// 2. Oversized skill (>20K tokens): injected as the largest whole-line
/// prefix that fits plus an `[INCOMPLETE SKILL]` notice; `offset=` is the
/// 1-based line right after the truncation point (read-tool alignment); the
/// dropped lines never reach the payload.
#[tokio::test]
async fn oversized_skill_body_truncates_with_continuation_notice() {
    // 5 lines x ~19K chars ≈ 23.7K tokens; 4 lines ≈ 19K fit.
    let line = |n: usize| format!("BIG-{n:02} {}", "x".repeat(19_000));
    let body = (0..5usize).map(line).collect::<Vec<_>>().join("\n");
    assert!(opencoder_llm::estimate(&body) > 20_000);

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _wd) = session_on("inj-big", "act", mock.clone());
    let path = "/skills/big/SKILL.md";
    s.set_skill(Some(sourced_body(path, &body)));

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let inj = injected(&s, path).expect("truncated injection still recorded");
    let text = inj.text();
    assert!(
        text.contains(&format!(
            "[INCOMPLETE SKILL] truncated at ~20K tokens; 1 lines remain; \
             read the rest with the read tool: read(path=\"{path}\", offset=5)."
        )),
        "notice names remaining lines + next offset: {}",
        &text[text.len().saturating_sub(220)..]
    );
    let cut = text.find("\n[INCOMPLETE SKILL]").expect("notice follows prefix");
    assert!(
        opencoder_llm::estimate(&text[..cut]) <= 20_000,
        "marker + truncated prefix stays within budget"
    );
    assert!(text[..cut].contains("BIG-03"), "lines 0..=3 kept");
    assert!(!text[..cut].contains("BIG-04"), "line 4 dropped");

    let req = &mock.requests()[0];
    assert!(any_user_contains(req, "BIG-03"));
    assert!(
        !any_user_contains(req, "BIG-04"),
        "truncated-away lines never reach the payload"
    );
}

/// 3. Switching skills appends a NEW injection (append-only transcript);
/// both bodies ride later payloads, and the old one is never deleted.
#[tokio::test]
async fn switching_skills_injects_new_entry_keeps_old() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("one")])
            .push_script(vec![done_turn("two")]),
    );
    let (mut s, _wd) = session_on("inj-switch", "act", mock.clone());
    s.set_skill(Some(sourced_body("/skills/a/SKILL.md", "A-BODY")));
    run(&mut s, "use A".into(), |_| {}).await.unwrap();

    s.set_skill(Some(sourced_body("/skills/b/SKILL.md", "B-BODY")));
    run(&mut s, "now B".into(), |_| {}).await.unwrap();

    assert!(injected(&s, "/skills/a/SKILL.md").is_some());
    assert!(injected(&s, "/skills/b/SKILL.md").is_some());
    assert_eq!(injected_count(&s), 2);
    let req2 = &mock.requests()[1];
    assert!(any_user_contains(req2, "A-BODY"), "old entry preserved");
    assert!(any_user_contains(req2, "B-BODY"), "new entry injected");
}

/// 4. Exclusions mirror `tail_reminder` gating: subagents (`explore`) and
/// the todos scheduler (`workflow`, itself Primary-mode) never get the
/// injection — transcript or payload.
#[tokio::test]
async fn subagent_and_workflow_never_get_injection() {
    for agent in ["explore", "workflow"] {
        let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
        let (mut s, _wd) = session_on(&format!("inj-excl-{agent}"), agent, mock.clone());
        s.set_skill(Some(sourced_body("/skills/x/SKILL.md", "X-BODY")));

        run(&mut s, "scoped".into(), |_| {}).await.unwrap();

        assert_eq!(injected_count(&s), 0, "{agent}: no transcript injection");
        let req = &mock.requests()[0];
        assert!(
            !any_user_contains(req, "[skill loaded]"),
            "{agent}: no payload injection"
        );
    }
}

/// 5. Legacy bodies without a `> Source:` prefix carry no path, hence no
/// injection — the transient `[active skill]` reminder doesn't fire either.
#[tokio::test]
async fn legacy_body_without_source_prefix_is_not_injected() {
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _wd) = session_on("inj-legacy", "act", mock.clone());
    s.set_skill(Some("LEGACY-BODY".into()));

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    assert_eq!(injected_count(&s), 0);
    assert!(!any_user_contains(&mock.requests()[0], "[skill loaded]"));
}
