//! Skill-context tail injection — integration contract tests.
//!
//! Skill content never ships in the system prompt anymore (`build_system`
//! takes no skill parameter): every LLM call derives one synthetic user
//! message appended at the END of the payload (`skill_context::tail_reminder`)
//! carrying (a) the `[skills]` catalog of config-enabled skills plus a
//! lazy-load hint and (b) the `[active skill]` source path parsed from the
//! body — a FALLBACK pointer that stays suppressed while the matching
//! `[skill loaded]` body message is already on record (F3)
//! `> Source:` prefix that `opencoder_core::body_with_source` writes. The
//! message is transient — never recorded into `session.messages` — and is
//! regenerated per call, so it survives compaction for free.
//!
//! Pinned against real request payloads captured by `MockChatClient`:
//! 1. prefix-cache stability: the system message stays byte-identical while
//!    catalog config and active-skill state flip mid-session;
//! 2. catalog reminder shape: final payload message, directory + entries +
//!    lazy-load guidance, disabled skills filtered out, never persisted;
//! 3. active-skill path reminder keeps the system prompt clean (and the
//!    legacy no-`> Source:` body parse contract);
//! 4. subagent/workflow exclusion.
//!
//! Every test flips `$HOME` (`PreparedHome` under a shared `HOME_LOCK`) so
//! `skills_dir()` resolves to a prepared tempdir with real skill files;
//! HOME is restored on drop (best-effort guard struct).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use opencoder_core::config::SkillConfig;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{run, SessionState};

/// Serializes every test in this binary: each flips `$HOME`, so skill
/// discovery must never observe another test's prepared tree.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Test fixture: takes `HOME_LOCK`, points `$HOME` at a tempdir carrying
/// real skill files under `.opencoder/skills/`, restores the previous HOME
/// on drop. Mirrors the `ScopedHome` pattern in `compact_none_over_budget`.
struct PreparedHome {
    _guard: std::sync::MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    prev: Option<std::ffi::OsString>,
}

impl PreparedHome {
    /// `<home>/.opencoder/skills` holding two real skills: `alpha`
    /// ("Alpha pack summary") and `beta` ("Beta pack summary") — layout per
    /// `opencoder_core::discover_in` docs.
    fn new() -> PreparedHome {
        let guard = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join(".opencoder").join("skills");
        for (name, description) in [
            ("alpha", "Alpha pack summary"),
            ("beta", "Beta pack summary"),
        ] {
            let pack = skills.join(name);
            std::fs::create_dir_all(&pack).unwrap();
            std::fs::write(
                pack.join("SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: {description}\n---\n{name}-BODY-CONTENT\n"
                ),
            )
            .unwrap();
        }
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        PreparedHome {
            _guard: guard,
            dir,
            prev,
        }
    }

    fn skills_root(&self) -> PathBuf {
        self.dir.path().join(".opencoder").join("skills")
    }

    fn skill_file(&self, name: &str) -> PathBuf {
        self.skills_root().join(name).join("SKILL.md")
    }
}

impl Drop for PreparedHome {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Config with the given skill names default-injection enabled.
fn config_with_skills(enabled: &[&str]) -> Config {
    let mut cfg = Config {
        model: "m/g".into(),
        ..Config::default()
    };
    for name in enabled {
        cfg.skills
            .insert((*name).to_string(), SkillConfig { enabled: true });
    }
    cfg
}

/// Session on the prepared HOME's skills tree. No store is attached:
/// persistence is a no-op, keeping assertions focused on request payloads.
/// The returned TempDir pins the working directory for the session's life.
fn session_on(
    id: &str,
    agent_name: &str,
    cfg: Config,
    client: Arc<MockChatClient>,
) -> (SessionState, tempfile::TempDir) {
    let workdir = tempfile::tempdir().unwrap();
    let session = SessionState::new(
        id,
        resolve_agent(agent_name).expect("builtin agent"),
        cfg,
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

/// Content of the LAST user-role message — where the transient skill-context
/// reminder is appended.
fn last_user_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

fn any_user_contains(req: &opencoder_llm::ChatRequest, needle: &str) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains(needle))
    })
}

fn any_message_contains(req: &opencoder_llm::ChatRequest, needle: &str) -> bool {
    req.messages.iter().any(|m| m.to_string().contains(needle))
}

/// 1. Prefix-cache stability: flipping the skills-catalog config AND the
/// active skill mid-session must not move a single byte of the system
/// message — skill context rides the payload tail instead, so provider
/// prompt-prefix caches survive skill activation.
#[tokio::test]
async fn system_prompt_bytes_stable_across_catalog_and_activation_changes() {
    let home = PreparedHome::new();
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("one")])
            .push_script(vec![done_turn("two")])
            .push_script(vec![done_turn("three")]),
    );
    let (mut s, _workdir) =
        session_on("prefix-cache", "act", config_with_skills(&[]), mock.clone());

    // Turn 1: no enabled catalog, no active skill → no reminder anywhere.
    run(&mut s, "first question".into(), |_| {}).await.unwrap();
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert!(!any_message_contains(&requests[0], "[skills]"));
    assert!(!any_message_contains(&requests[0], "[active skill]"));

    // Mutate the session mid-flight: enable `alpha` in the config catalog
    // AND activate a Source-prefixed skill (both mutable session state).
    s.config
        .skills
        .insert("alpha".into(), SkillConfig { enabled: true });
    s.set_skill(Some(format!(
        "> Source: {}\n\nalpha-BODY-CONTENT",
        home.skill_file("alpha").display()
    )));
    run(&mut s, "second question".into(), |_| {}).await.unwrap();

    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let (first, second) = (&requests[0], &requests[1]);

    // The contract: byte-identical system messages across the flip.
    assert_eq!(system_content(first), system_content(second));
    assert!(
        !system_content(second).contains("BODY-CONTENT"),
        "skill bodies never ship in the system prompt"
    );

    // The payload tail DID change: the catalog section appears, and the
    // `[active skill]` pointer stays suppressed because turn 2 already
    // carries the in-conversation `[skill loaded]` body message.
    let tail = last_user_content(second);
    assert!(tail.contains("[skills]"), "{tail}");
    assert!(!tail.contains("[active skill]"), "{tail}");
    assert_ne!(tail, last_user_content(first));
    assert!(
        any_message_contains(second, "[skill loaded]")
            && any_message_contains(second, "alpha-BODY-CONTENT"),
        "the active skill body ships via the loaded message: {:?}",
        second.messages
    );

    // Toggle everything back OFF: three-way byte stability.
    s.config.skills.clear();
    s.set_skill(None);
    run(&mut s, "third question".into(), |_| {}).await.unwrap();
    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(system_content(first), system_content(&requests[2]));
    assert_eq!(system_content(second), system_content(&requests[2]));
}

/// 2. Catalog reminder shape: with `alpha` enabled (and `beta` present on
/// disk but disabled), the request's LAST message is the transient
/// `[skills]` reminder — the real user text is no longer final. It names the
/// skills directory, lists `- alpha: Alpha pack summary`, carries the
/// lazy-load guidance, omits the disabled `beta`, and never lands in
/// `session.messages`.
#[tokio::test]
async fn skills_catalog_reminder_is_last_payload_message_and_never_persisted() {
    let home = PreparedHome::new();
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _workdir) = session_on(
        "catalog",
        "act",
        config_with_skills(&["alpha"]),
        mock.clone(),
    );

    run(
        &mut s,
        "please inventory the available skills".into(),
        |_| {},
    )
    .await
    .unwrap();

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];

    // The reminder is the final message of the payload: nothing after it.
    let last = req.messages.last().expect("non-empty payload");
    assert_eq!(
        last.get("role").and_then(|r| r.as_str()),
        Some("user"),
        "the reminder must be the last message: {last:?}"
    );
    let tail = last.get("content").and_then(|c| c.as_str()).unwrap_or("");
    assert!(tail.starts_with("[skills]"), "{tail}");
    assert!(
        tail.contains(&home.skills_root().display().to_string()),
        "must name the skills directory: {tail}"
    );
    assert!(tail.contains("- alpha: Alpha pack summary"), "{tail}");
    assert!(
        tail.contains("read its SKILL.md file"),
        "lazy-load guidance: {tail}"
    );
    assert!(
        !tail.contains("beta"),
        "disabled skills stay out of the catalog: {tail}"
    );

    // The real user text is NOT the last message anymore — it rides earlier.
    assert!(!tail.contains("please inventory"));
    assert!(any_user_contains(req, "please inventory"));

    // Transient: derived per call, never recorded into the transcript.
    assert!(
        !s.messages.iter().any(|m| m.text().contains("[skills]")),
        "the reminder must never persist into session.messages"
    );
}

/// 3a. Active-skill delivery: a `> Source:`-prefixed body reaches the model
/// as the in-conversation `[skill loaded]` message; the `[active skill]`
/// tail pointer is fallback-only and stays off while that marker is on
/// record — the body text stays out of the system message either way.
#[tokio::test]
async fn active_skill_body_ships_via_loaded_message_and_keeps_system_clean() {
    let home = PreparedHome::new();
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _workdir) = session_on("active", "act", config_with_skills(&[]), mock.clone());

    let source = home.skill_file("alpha");
    s.set_skill(Some(format!(
        "> Source: {}\n\nalpha-BODY-CONTENT",
        source.display()
    )));

    run(&mut s, "follow the active skill".into(), |_| {})
        .await
        .unwrap();

    let req = &mock.requests()[0];
    assert!(
        !system_content(req).contains("alpha-BODY-CONTENT"),
        "skill bodies never ship in the system prompt"
    );
    let tail = last_user_content(req);
    assert!(
        !tail.contains("[active skill]"),
        "pointer suppressed while the loaded marker is on record: {tail}"
    );
    assert!(
        tail.contains("[skill loaded]") && tail.contains(&source.display().to_string()),
        "the loaded message names the skill's source file: {tail}"
    );
    assert!(
        tail.contains("alpha-BODY-CONTENT"),
        "the body ships once, inside the loaded message: {tail}"
    );
    assert!(
        !system_content(req).contains("[active skill]"),
        "the pointer never leaks into the system prompt"
    );
}

/// 3b. Parse contract: a legacy body WITHOUT the `> Source:` prefix yields
/// NO `[active skill]` section at all (there is no path to point at); with
/// an empty config catalog there is no `[skills]` section either.
#[tokio::test]
async fn legacy_body_without_source_prefix_yields_no_active_skill_section() {
    let _home = PreparedHome::new();
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
    let (mut s, _workdir) = session_on("legacy", "act", config_with_skills(&[]), mock.clone());

    s.set_skill(Some("LEGACY-BODY-WITHOUT-SOURCE-PREFIX".into()));
    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let req = &mock.requests()[0];
    assert!(!any_message_contains(req, "[active skill]"));
    assert!(!any_message_contains(req, "[skills]"));
    assert!(
        !system_content(req).contains("LEGACY-BODY-WITHOUT-SOURCE-PREFIX"),
        "legacy bodies are not shipped anywhere either"
    );
}

/// 4. Exclusion: subagents (`explore`) and the todos scheduler (`workflow`,
/// itself Primary-mode) never receive skill context — no `[skills]` catalog
/// and no `[active skill]` reminder anywhere in their payloads, even with an
/// enabled config skill and an active Source-prefixed skill.
#[tokio::test]
async fn subagent_and_workflow_payloads_carry_no_skill_context() {
    let home = PreparedHome::new();
    for agent in ["explore", "workflow"] {
        let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("ok")]));
        let (mut s, _workdir) = session_on(
            &format!("exclude-{agent}"),
            agent,
            config_with_skills(&["alpha"]),
            mock.clone(),
        );
        s.set_skill(Some(format!(
            "> Source: {}\n\nalpha-BODY-CONTENT",
            home.skill_file("alpha").display()
        )));

        run(&mut s, "scoped task".into(), |_| {}).await.unwrap();

        let req = &mock.requests()[0];
        assert!(
            !any_message_contains(req, "[skills]"),
            "{agent} must not receive the skills catalog: {:?}",
            req.messages
        );
        assert!(
            !any_message_contains(req, "[active skill]"),
            "{agent} must not receive the active-skill reminder: {:?}",
            req.messages
        );
    }
}
