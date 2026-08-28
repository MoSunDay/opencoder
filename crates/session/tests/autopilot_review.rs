//! Integration tests for the three-state autopilot mode dispatch through the
//! `run` entry point, focused on the one-shot review pass
//! (`autopilot.mode = "review"`): exactly one `Review` marker + one review
//! turn (never a PLAN/ACT/VERIFY loop), the agent-agnostic dispatch (the pass
//! runs on whatever primary agent completed the task — no switch), review
//! skill activation from `~/.opencoder/skills` and its cleanup, plus the `ap`
//! regression (phases still cycle under `max_iterations = 1`).
//!
//! Split out of `autopilot.rs` to keep both files within the per-file line
//! budget; helpers are deliberately duplicated (the shared `tests/common`
//! module is resume-fixture specific).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, ApMode, AutoPilotConfig, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::autopilot::{review_pass, ApPhase};
use opencoder_session::runner::run_with_registry;
use opencoder_session::tools::registry;
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store as StoreTrait};

/// A completed idle turn (no tool calls).
fn completed(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

/// Config with the given autopilot mode. `..AutoPilotConfig::default()` keeps
/// the literal future-proof against new fields (same pattern as
/// `autopilot_config` in `autopilot.rs`).
fn mode_config(mode: ApMode, max_iterations: u32) -> Config {
    Config {
        model: "m/g".into(),
        autopilot: AutoPilotConfig {
            mode,
            max_iterations,
            ..AutoPilotConfig::default()
        },
        ..Config::default()
    }
}

fn make_session(mock: Arc<dyn ChatStream>, config: Config) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        "ap-mode-sess",
        agent,
        config,
        mock,
        dir.path().to_path_buf(),
    );
    (dir, s)
}

fn collector() -> (Arc<Mutex<Vec<SessionEvent>>>, impl FnMut(SessionEvent)) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let clone = buf.clone();
    let f = move |ev: SessionEvent| clone.lock().unwrap().push(ev);
    (buf, f)
}

fn phase_label(phase: &ApPhase) -> &'static str {
    match phase {
        ApPhase::Plan => "plan",
        ApPhase::Act => "act",
        ApPhase::Verify => "verify",
        ApPhase::Review => "review",
    }
}

/// True when any message of the captured request contains `needle`
/// (messages are JSON values; stringified for the substring scan).
fn any_message_contains(req: &opencoder_llm::ChatRequest, needle: &str) -> bool {
    req.messages.iter().any(|m| m.to_string().contains(needle))
}

// ── HOME isolation: skill discovery reads the process $HOME ───────────────
//
// `activate_review_skill` discovers `~/.opencoder/skills` via
// `opencoder_core::skill::skills_dir()`, which resolves the *process* HOME
// through `dirs::home_dir()` — the thread-local `scoped_config_home` only
// redirects config discovery, not skill discovery. So the review-skill test
// flips `$HOME` under a mutex, mirroring `lock_home` in `control_cmd.rs`.

static HOME_MUTEX: Mutex<()> = Mutex::new(());

/// RAII guard pointing `$HOME` at `home`; drop restores the previous value.
struct HomeGuard {
    prev: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn lock_home(home: &Path) -> HomeGuard {
    let _lock = HOME_MUTEX.lock().unwrap();
    let prev = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    HomeGuard { prev, _lock }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// Write a minimal `review` skill (frontmatter `name`/`description` + body,
/// per `opencoder_core::skill::parse_skill`) under
/// `<home>/.opencoder/skills/review/SKILL.md`; returns that SKILL.md path.
fn seed_review_skill(home: &Path) -> PathBuf {
    let pack = home.join(".opencoder").join("skills").join("review");
    std::fs::create_dir_all(&pack).unwrap();
    let skill_md = pack.join("SKILL.md");
    std::fs::write(
        &skill_md,
        "---\nname: review\ndescription: Review the completed work\n---\nREVIEW-SKILL-BODY\n",
    )
    .unwrap();
    skill_md
}

// ── review mode: one-shot pass, never a loop ──────────────────────────────

/// mode=Review: the initial task completes, then exactly one review turn
/// runs on the CURRENT agent (no switch) and the run finishes — the pass
/// must NOT continue into a PLAN/ACT/VERIFY cycle.
#[tokio::test]
async fn review_mode_runs_exactly_one_review_pass() {
    // initial task (1 call) + review turn (1 call).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("initial")])
            .push_script(vec![completed("review-0")]),
    );
    let (_dir, mut session) =
        make_session(mock as Arc<dyn ChatStream>, mode_config(ApMode::Review, 10));

    let reg = registry();
    let (buf, mut on_event) = collector();
    run_with_registry(&mut session, "do it".into(), vec![], &reg, &mut on_event)
        .await
        .unwrap();
    let events = buf.lock().unwrap().clone();

    // (i) exactly one AutoPilot { Review, 0 } marker — zero-based, matching
    // drive's ApState::iteration which also starts at 0.
    let review_count = events
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                SessionEvent::AutoPilot {
                    phase: ApPhase::Review,
                    iteration: 0
                }
            )
        })
        .count();
    assert_eq!(review_count, 1, "exactly one AutoPilot(Review, 0) event");

    // (ii) the pass makes NO agent switch — the reviewer stays on whatever
    // agent completed the task.
    assert!(
        events
            .iter()
            .all(|ev| !matches!(ev, SessionEvent::AgentSwitch(_))),
        "review pass is agent-agnostic: no AgentSwitch, got {events:?}"
    );

    // (iii) a terminal Done follows the review marker.
    let review_idx = events
        .iter()
        .position(|ev| {
            matches!(
                ev,
                SessionEvent::AutoPilot {
                    phase: ApPhase::Review,
                    ..
                }
            )
        })
        .expect("review marker present");
    assert!(
        events[review_idx..]
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Done)),
        "a Done event must follow the review pass"
    );

    // (iv) no PLAN/ACT/VERIFY phase events — the pass never loops.
    let loop_phases: Vec<&'static str> = events
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::AutoPilot { phase, .. } => match phase {
                ApPhase::Plan | ApPhase::Act | ApPhase::Verify => Some(phase_label(phase)),
                ApPhase::Review => None,
            },
            _ => None,
        })
        .collect();
    assert!(
        loop_phases.is_empty(),
        "review mode must not run the PLAN/ACT/VERIFY loop, got {loop_phases:?}"
    );
}

/// mode=Review with a discoverable `review` skill: the review turn's request
/// carries the `[active skill]` reminder pointing at the seeded SKILL.md, and
/// the skill is cleared once the pass finishes (it must not leak into the
/// next user turn).
#[tokio::test]
async fn review_mode_activates_then_clears_review_skill() {
    let home = tempfile::tempdir().unwrap();
    let skill_md = seed_review_skill(home.path());
    let _guard = lock_home(home.path());

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("initial")])
            .push_script(vec![completed("review-0")]),
    );
    let (_dir, mut session) = make_session(
        mock.clone() as Arc<dyn ChatStream>,
        mode_config(ApMode::Review, 10),
    );

    let reg = registry();
    let (_buf, mut on_event) = collector();
    run_with_registry(&mut session, "do it".into(), vec![], &reg, &mut on_event)
        .await
        .unwrap();

    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "initial turn + one review turn");

    // The initial (act) turn runs before the skill exists on the session.
    assert!(
        !any_message_contains(&requests[0], "[active skill]"),
        "no skill may be active during the initial task"
    );
    // The review turn carries the active-skill reminder with the seeded path.
    assert!(
        any_message_contains(&requests[1], "[active skill]"),
        "the review skill must be active for the review turn"
    );
    assert!(
        any_message_contains(&requests[1], &skill_md.to_string_lossy()),
        "the reminder must point at the seeded review SKILL.md"
    );

    // Cleanup: the skill does not outlive the pass.
    assert!(
        session.skill_prompt_cloned().is_none(),
        "review skill must be cleared after the pass"
    );
}

/// mode=Review under the sandbox agent: the dispatch is agent-agnostic — the
/// read-only reviewer runs on the sandbox agent exactly like on act: two LLM
/// calls (initial + review), the Review marker, and the synthetic review
/// prompt in the transcript.
#[tokio::test]
async fn review_mode_runs_pass_in_sandbox_mode() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("explored")])
            .push_script(vec![completed("review-0")]),
    );
    let (_dir, mut session) = {
        let dir = tempfile::tempdir().unwrap();
        let agent = resolve_agent("sandbox").unwrap();
        let s = SessionState::new(
            "ap-sandbox-mode-sess",
            agent,
            mode_config(ApMode::Review, 10),
            mock.clone() as Arc<dyn ChatStream>,
            dir.path().to_path_buf(),
        );
        (dir, s)
    };

    let reg = registry();
    let (buf, mut on_event) = collector();
    run_with_registry(
        &mut session,
        "explore it".into(),
        vec![],
        &reg,
        &mut on_event,
    )
    .await
    .unwrap();
    let events = buf.lock().unwrap().clone();

    // (i) two LLM calls — the initial turn + the review turn.
    assert_eq!(
        mock.call_count(),
        2,
        "the pass must run after the initial sandbox turn"
    );
    // (ii) the Review marker was emitted.
    assert!(
        events.iter().any(|ev| matches!(
            ev,
            SessionEvent::AutoPilot {
                phase: ApPhase::Review,
                ..
            }
        )),
        "sandbox mode must dispatch the review pass"
    );
    // (iii) no agent switch anywhere.
    assert!(
        events
            .iter()
            .all(|ev| !matches!(ev, SessionEvent::AgentSwitch(_))),
        "no agent switch may happen in sandbox mode, got {events:?}"
    );
    // (iv) the synthetic review prompt landed in the transcript.
    assert!(
        session
            .messages
            .iter()
            .any(|m| message_text(m).contains("Review the work completed")),
        "the synthetic review prompt must be recorded"
    );
}

/// Concatenated text blocks of a transcript message (for transcript scans).
fn message_text(m: &opencoder_core::Message) -> String {
    m.blocks
        .iter()
        .filter_map(|b| match b {
            opencoder_core::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

// ── ap mode regression through the run entry ──────────────────────────────

/// mode=Ap still cycles Plan -> Act -> Verify even when clamped to a single
/// iteration (verify says "no" -> MaxIterations after one full cycle).
#[tokio::test]
async fn ap_mode_with_max_iterations_one_still_cycles_phases() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("initial")])
            .push_script(vec![completed("plan-0")])
            .push_script(vec![completed("act-0")])
            .push_script(vec![completed("no")]),
    );
    let (_dir, mut session) = make_session(mock as Arc<dyn ChatStream>, mode_config(ApMode::Ap, 1));

    let reg = registry();
    let (buf, mut on_event) = collector();
    run_with_registry(&mut session, "do it".into(), vec![], &reg, &mut on_event)
        .await
        .unwrap();

    let phases: Vec<&'static str> = buf
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::AutoPilot { phase, .. } => Some(phase_label(phase)),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec!["plan", "act", "verify"],
        "mode=Ap must run one full phase cycle under max_iterations=1"
    );

    // Drive-path 0-based iteration anchor: `ApState::iteration` starts at 0,
    // so every event of the FIRST cycle carries iteration == 0 — the same
    // convention `review_pass` now follows (`should_stop`'s
    // `iteration + 1 >= max` cap arithmetic already assumes it).
    let iterations: Vec<u32> = buf
        .lock()
        .unwrap()
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::AutoPilot { iteration, .. } => Some(*iteration),
            _ => None,
        })
        .collect();
    assert_eq!(iterations, vec![0, 0, 0], "drive iterations are 0-based");
}

/// mode=Review with an attached store: an LLM failure during the review turn
/// (e.g. 429 exhaustion) must still run the terminal bookkeeping before the
/// error propagates — skill cleared in memory AND persisted
/// (`sessions.skill` -> NULL) — or a resume after the error would resurrect
/// the system-injected review skill that the user never asked for.
#[tokio::test]
async fn review_mode_error_still_clears_and_persists_skill() {
    let home = tempfile::tempdir().unwrap();
    seed_review_skill(home.path());
    let _guard = lock_home(home.path());

    // Initial turn completes; the review turn dies with a stream error.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![completed("initial")])
            .push_script(vec![LlmEvent::Error("429 rate limited".into())]),
    );
    let (_dir, mut session) =
        make_session(mock as Arc<dyn ChatStream>, mode_config(ApMode::Review, 10));

    // Attach a store whose session row carries a stale skill body — exactly
    // what a resume (or any skill-persisting path) would have written.
    let store: Arc<dyn StoreTrait> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "ap-mode-sess".into(),
            agent: Some("act".into()),
            skill: Some("STALE-SKILL-BODY".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    session.store = Some(store.clone());

    let reg = registry();
    let (buf, mut on_event) = collector();
    let res = run_with_registry(&mut session, "do it".into(), vec![], &reg, &mut on_event).await;
    assert!(res.is_err(), "the review-turn LLM error must propagate");

    // In-memory clear held despite the error...
    assert!(
        session.skill_prompt_cloned().is_none(),
        "review skill must be cleared even when the review turn errors"
    );
    // ...the uniform end marker is still emitted...
    assert!(
        buf.lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Done)),
        "terminal Done event must be emitted on a review-turn error"
    );
    // ...and the clear is durable: the store row no longer carries a skill.
    let stored = store
        .get_session("ap-mode-sess")
        .await
        .unwrap()
        .expect("session row exists");
    assert!(
        stored.skill.is_none(),
        "the skill clear must be persisted, got {:?}",
        stored.skill
    );
}

/// A cancel tripped BEFORE the pass starts is a complete no-op: no review
/// marker, no agent switch, no synthetic review prompt, no LLM call — just
/// the uniform terminal Done (mirroring drive's loop-top cancel path).
#[tokio::test]
async fn review_pass_cancelled_at_entry_is_a_no_op() {
    let mock = Arc::new(MockChatClient::new());
    let (_dir, mut session) = make_session(
        mock.clone() as Arc<dyn ChatStream>,
        mode_config(ApMode::Review, 10),
    );
    session
        .record(opencoder_core::Message::user("u1", "do the thing"))
        .await;
    let messages_before = session.messages.len();
    let token = tokio_util::sync::CancellationToken::new();
    session = session.with_cancel(token.clone());
    token.cancel();

    let reg = registry();
    let (buf, mut on_event) = collector();
    review_pass(&mut session, &reg, &mut on_event)
        .await
        .expect("a cancelled entry returns Ok, mirroring drive's cancel path");

    assert_eq!(
        mock.call_count(),
        0,
        "cancelled entry must not call the LLM"
    );
    assert_eq!(
        session.agent.name, "act",
        "agent must stay unchanged (review never switches)"
    );
    assert_eq!(
        session.messages.len(),
        messages_before,
        "no synthetic review prompt may be recorded"
    );
    assert!(
        !session.messages.iter().any(|m| m.synthetic),
        "no synthetic message of any kind"
    );
    let events = buf.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .all(|ev| !matches!(ev, SessionEvent::AutoPilot { .. })),
        "no AutoPilot marker on a cancelled entry, got {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|ev| !matches!(ev, SessionEvent::AgentSwitch(_))),
        "no AgentSwitch on a cancelled entry, got {events:?}"
    );
    assert_eq!(
        events.len(),
        1,
        "exactly one event — the uniform Done, got {events:?}"
    );
    assert!(matches!(events[0], SessionEvent::Done));
}
