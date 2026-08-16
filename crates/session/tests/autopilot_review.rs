//! Integration tests for the three-state autopilot mode dispatch through the
//! `run` entry point, focused on the one-shot review pass
//! (`autopilot.mode = "review"`): exactly one `Review` marker + one review
//! turn (never a PLAN/ACT/VERIFY loop), the switch to the plan agent,
//! review-skill activation from `~/.opencoder/skills` and its cleanup, plus
//! the `ap` regression (phases still cycle under `max_iterations = 1`).
//!
//! Split out of `autopilot.rs` to keep both files within the per-file line
//! budget; helpers are deliberately duplicated (the shared `tests/common`
//! module is resume-fixture specific).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, ApMode, AutoPilotConfig, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::autopilot::ApPhase;
use opencoder_session::runner::run_with_registry;
use opencoder_session::tools::registry;
use opencoder_session::{SessionEvent, SessionState};

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
    let s = SessionState::new("ap-mode-sess", agent, config, mock, dir.path().to_path_buf());
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
    req.messages
        .iter()
        .any(|m| m.to_string().contains(needle))
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
/// runs under the plan agent and the run finishes — the pass must NOT
/// continue into a PLAN/ACT/VERIFY cycle.
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

    // (i) exactly one AutoPilot { Review, 1 } marker.
    let review_count = events
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                SessionEvent::AutoPilot {
                    phase: ApPhase::Review,
                    iteration: 1
                }
            )
        })
        .count();
    assert_eq!(review_count, 1, "exactly one AutoPilot(Review, 1) event");

    // (ii) the pass switched to the plan agent.
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, SessionEvent::AgentSwitch(name) if name == "plan")),
        "review pass must switch to the plan agent"
    );

    // (iii) a terminal Done follows the review marker.
    let review_idx = events
        .iter()
        .position(|ev| matches!(ev, SessionEvent::AutoPilot { phase: ApPhase::Review, .. }))
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
    let (_dir, mut session) =
        make_session(mock.clone() as Arc<dyn ChatStream>, mode_config(ApMode::Review, 10));

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
    let (_dir, mut session) =
        make_session(mock as Arc<dyn ChatStream>, mode_config(ApMode::Ap, 1));

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
}
