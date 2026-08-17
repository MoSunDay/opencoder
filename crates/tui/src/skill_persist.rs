//! Persisting the active skill so it survives resume/restart.
//!
//! When a `$skill` token is submitted inline — most importantly when it is
//! **queued** via Tab while a turn is running — `resolve_and_warn` activates the
//! skill in-memory (`SessionState::skill_prompt`) but, unlike the skill-menu
//! (`SetSkill`) path, it never wrote the skill body to the store. A combined
//! submission like `$skill fix the bug` therefore queued/persisted only the
//! clean task text, so on resume the queued task ran **without** the skill.
//!
//! [`persist_skill`] mirrors the `SetSkill` path: a best-effort
//! `update_session(skill=…)` whenever the skill body changed.

use std::path::Path;
use std::sync::{Arc, Mutex};

use opencoder_core::message::now_ms;
use opencoder_store::{SessionPatch, Store};

use crate::app_helpers::resolve_and_warn_with;
use crate::chat::ChatView;

/// Persist the active skill to the store when it differs from `prev`.
///
/// `prev` must be the skill body captured *before* a submit/steer/queue's
/// `resolve_and_warn` call; `skill_handle` carries the body *after* (it is the
/// same `Arc<Mutex<Option<String>>>` shared with `SessionState::skill_prompt`).
/// When the two are equal — e.g. a plain-text submit with no skill token, or a
/// token that re-activates an already-persisted skill — this is a no-op.
///
/// Best-effort: store errors are swallowed, because the in-memory write that
/// `resolve_and_warn` already performed keeps the in-flight turn correct.
pub(crate) async fn persist_skill(
    store: &Arc<dyn Store>,
    session_id: &str,
    prev: &Option<String>,
    skill_handle: &Arc<Mutex<Option<String>>>,
) {
    let cur = skill_handle
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if cur.as_deref() == prev.as_deref() {
        return;
    }
    let _ = store
        .update_session(
            session_id,
            &SessionPatch {
                skill: cur,
                updated_at: Some(now_ms()),
                ..Default::default()
            },
        )
        .await;
}

/// Resolve `$skill` tokens in `text` (activating the skill in-memory) **and**
/// persist the result to the store when it changed — the single composition
/// `run_app` relies on for every Submit / Steer / Queue. Extracted so the
/// three call sites stay byte-identical and the wiring itself is testable.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_persist(
    text: &str,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
    chat: &mut ChatView,
    store: &Arc<dyn Store>,
    session_id: &str,
) -> (String, Vec<String>) {
    let skills = opencoder_core::discover_skills();
    resolve_persist_with(
        &skills,
        text,
        active_skill,
        active_skill_body,
        sys_tokens,
        agent_name,
        workdir,
        skill_handle,
        chat,
        store,
        session_id,
    )
    .await
}

/// [`resolve_persist`] resolved against an *explicit* skill slice (typically
/// `discover_in(tempdir)`), so tests avoid mutating the process-global `HOME`
/// that `discover_skills()` reads. See [`resolve_and_warn_with`].
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_persist_with(
    skills: &[opencoder_core::Skill],
    text: &str,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
    chat: &mut ChatView,
    store: &Arc<dyn Store>,
    session_id: &str,
) -> (String, Vec<String>) {
    let prev = skill_handle
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let (clean, unresolved) = resolve_and_warn_with(
        skills,
        text,
        active_skill,
        active_skill_body,
        sys_tokens,
        agent_name,
        workdir,
        skill_handle,
        chat,
    );
    persist_skill(store, session_id, &prev, skill_handle).await;
    (clean, unresolved)
}

/// Apply a `KeyAction::SetSkill` selection: update the sticky in-memory skill
/// state (active name/body, token estimate, shared skill mutex) and persist
/// the selection (best-effort) so it survives resume/restart. The in-memory
/// mutex write keeps the in-flight turn immediate.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_skill_selection(
    opt: &Option<(String, String)>,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
    store: &Arc<dyn Store>,
    session_id: &str,
) {
    let skill_body = opt.as_ref().map(|(_, body)| body.clone());
    match opt {
        Some((name, body)) => {
            *active_skill = Some(name.clone());
            *active_skill_body = Some(body.clone());
            *sys_tokens = crate::app_helpers::sys_tokens_for(agent_name, workdir, Some(body));
            *skill_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(body.clone());
        }
        None => {
            *active_skill = None;
            *active_skill_body = None;
            *sys_tokens = crate::app_helpers::sys_tokens_for(agent_name, workdir, None);
            *skill_handle.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    // Store semantics: `skill: None` in a patch means "don't touch", so a
    // clear MUST go through the explicit `clear_skill` flag — a plain
    // `skill: None` write would silently no-op and the skill would
    // resurrect on resume (fake persistence).
    let patch = if skill_body.is_some() {
        SessionPatch {
            skill: skill_body,
            updated_at: Some(now_ms()),
            ..Default::default()
        }
    } else {
        SessionPatch {
            clear_skill: true,
            updated_at: Some(now_ms()),
            ..Default::default()
        }
    };
    let _ = store.update_session(session_id, &patch).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::{LibsqlStore, SessionMeta};

    async fn fresh_store() -> Arc<dyn Store> {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "s".into(),
                agent: Some("act".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        store
    }

    fn handle(v: Option<&str>) -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(v.map(String::from)))
    }

    /// Mirrors a combined `$skill do X` submit: the token resolves and sets
    /// `skill_prompt` to `Some(body)` while `prev` (the pre-submit body) was
    /// `None`. The skill must land in the store so a later resume restores it.
    #[tokio::test]
    async fn persist_skill_writes_newly_activated_skill() {
        let store = fresh_store().await;
        // Before resolve: no skill (prev). After resolve: body activated.
        let skill_handle = handle(Some("the skill body"));
        persist_skill(&store, "s", &None, &skill_handle).await;

        let persisted = store.get_session("s").await.unwrap().unwrap();
        assert_eq!(persisted.skill.as_deref(), Some("the skill body"));
    }

    /// Re-activating the *same* already-persisted skill is a no-op: prev equals
    /// the current body, so we don't churn a redundant store write.
    #[tokio::test]
    async fn persist_skill_skips_unchanged_skill() {
        let store = fresh_store().await;
        // Seed the store as if the menu path already persisted this skill.
        let _ = store
            .update_session(
                "s",
                &SessionPatch {
                    skill: Some("the skill body".into()),
                    updated_at: Some(1),
                    ..Default::default()
                },
            )
            .await;
        let before = store.get_session("s").await.unwrap().unwrap().updated_at;

        let skill_handle = handle(Some("the skill body"));
        persist_skill(&store, "s", &Some("the skill body".into()), &skill_handle).await;

        let after = store.get_session("s").await.unwrap().unwrap();
        assert_eq!(after.skill.as_deref(), Some("the skill body"));
        assert_eq!(
            after.updated_at, before,
            "unchanged skill must not trigger a store write"
        );
    }

    /// A plain-text submit with no skill token leaves both prev and the handle
    /// at `None`; the skill must stay absent rather than be wiped or written.
    #[tokio::test]
    async fn persist_skill_noop_when_no_skill_token() {
        let store = fresh_store().await;
        let skill_handle = handle(None);
        persist_skill(&store, "s", &None, &skill_handle).await;

        let persisted = store.get_session("s").await.unwrap().unwrap();
        assert!(persisted.skill.is_none());
    }

    /// Regression for the reported bug: a **queued** `$skill fix the bug`
    /// submission. `extract_skill_tokens` strips the token and yields the clean
    /// task text; the resolved skill body lands in `skill_prompt`. Previously
    /// that body was in-memory only, so on resume the queued task ran with no
    /// skill. Now `persist_skill` carries it into the store row.
    #[tokio::test]
    async fn persist_skill_survives_combined_queued_skill_submission() {
        use opencoder_core::extract_skill_tokens;

        let store = fresh_store().await;
        let raw = "$repo-memory fix the bug";
        let (clean, names) = extract_skill_tokens(raw);

        // Mirror resolve_and_warn: token parsed, clean task text carried, skill
        // body activated in the (shared) skill_prompt handle.
        assert_eq!(names, vec!["repo-memory"]);
        assert_eq!(clean.trim(), "fix the bug");
        let skill_handle = handle(Some("the repo-memory skill body"));

        persist_skill(&store, "s", &None, &skill_handle).await;

        // The clean text is what a queue row would store; the skill lives on the
        // session row — exactly what resume reads back via meta.skill.
        assert_eq!(
            store
                .get_session("s")
                .await
                .unwrap()
                .unwrap()
                .skill
                .as_deref(),
            Some("the repo-memory skill body"),
            "queued combined skill+text must persist the skill for resume"
        );
    }

    /// Switching skills mid-session (`$a` then later `$b`) persists the new
    /// body, not the old one.
    #[tokio::test]
    async fn persist_skill_updates_when_skill_changes() {
        let store = fresh_store().await;
        let skill_handle = handle(Some("body-b"));
        persist_skill(&store, "s", &Some("body-a".into()), &skill_handle).await;

        let persisted = store.get_session("s").await.unwrap().unwrap();
        assert_eq!(persisted.skill.as_deref(), Some("body-b"));
    }

    /// End-to-end composition: `$alpha fix the bug` through `resolve_persist`
    /// activates the skill in-memory AND persists it to the store — the exact
    /// wiring `run_app`'s Submit/Steer/Queue branches rely on. Pins that the
    /// three call sites' "snapshot -> resolve -> persist" sequence is correct,
    /// not just the two halves in isolation.
    #[tokio::test]
    async fn resolve_persist_activates_and_stores_combined_skill_token() {
        // A tempdir holding a discoverable skill file. We pass it explicitly via
        // `discover_in` + `resolve_persist_with`, so no `HOME` mutation is
        // needed (the former `set_var("HOME", …)` is thread-unsafe → UB under
        // parallel test execution and would flake unrelated pure tests).
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".opencoder").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("alpha.md"), "the alpha body").unwrap();
        let skills = opencoder_core::discover_in(&skills_dir);

        let store = fresh_store().await;
        let skill_handle = handle(None);
        let mut active_skill = None;
        let mut active_skill_body = None;
        let mut sys_tokens = 0u64;
        let mut chat = crate::chat::ChatView {
            agent: "act".into(),
            ..Default::default()
        };
        let workdir = std::path::PathBuf::from("/tmp");

        let (clean, unresolved) = resolve_persist_with(
            &skills,
            "$alpha fix the bug",
            &mut active_skill,
            &mut active_skill_body,
            &mut sys_tokens,
            "act",
            &workdir,
            &skill_handle,
            &mut chat,
            &store,
            "s",
        )
        .await;

        // Clean text carries the task; token resolved; skill activated + persisted.
        assert_eq!(clean.trim(), "fix the bug");
        assert!(unresolved.is_empty());
        assert_eq!(active_skill.as_deref(), Some("alpha"));
        {
            let handle_body = skill_handle.lock().unwrap();
            let handle_body = handle_body.as_deref().expect("skill_prompt body set");
            assert!(
                handle_body.starts_with("> Source: "),
                "must prefix source path: {handle_body}"
            );
            assert!(
                handle_body.ends_with("the alpha body"),
                "skill_prompt (in-memory) must carry the resolved body: {handle_body}"
            );
        }
        let stored = store
            .get_session("s")
            .await
            .unwrap()
            .unwrap()
            .skill
            .clone()
            .expect("persisted skill body");
        assert!(
            stored.starts_with("> Source: "),
            "must prefix source path: {stored}"
        );
        assert!(
            stored.ends_with("the alpha body"),
            "resolve_persist must persist the skill for resume: {stored}"
        );
    }

    /// The `$`-menu clear row routes through `apply_skill_selection(None)`.
    /// The clear must reach the store via `clear_skill: true` — a plain
    /// `skill: None` patch is a "don't touch" no-op, so before the fix the
    /// skill resurrected on resume despite the in-memory clear (fake
    /// persistence). Seeds the store with an active skill first, exactly
    /// like a resumed sticky session.
    #[tokio::test]
    async fn apply_skill_selection_none_persists_clear() {
        let store = fresh_store().await;
        // A persisted sticky skill, as a resume or `$pick` would leave it.
        let _ = store
            .update_session(
                "s",
                &SessionPatch {
                    skill: Some("stale sticky body".into()),
                    updated_at: Some(1),
                    ..Default::default()
                },
            )
            .await;
        let skill_handle = handle(Some("stale sticky body"));
        let mut active_skill = Some("review".to_string());
        let mut active_skill_body = Some("stale sticky body".to_string());
        let mut sys_tokens = 42u64;
        let mut chat = crate::chat::ChatView::default();

        apply_skill_selection(
            &None,
            &mut active_skill,
            &mut active_skill_body,
            &mut sys_tokens,
            "act",
            std::path::Path::new("/tmp"),
            &skill_handle,
            &store,
            "s",
        )
        .await;

        // In-memory sticky state is wiped.
        assert!(active_skill.is_none());
        assert!(active_skill_body.is_none());
        assert!(skill_handle.lock().unwrap().is_none());
        // ...and the clear is durable: the store row no longer carries the
        // skill, so a resume must not resurrect it.
        let persisted = store.get_session("s").await.unwrap().unwrap();
        assert!(
            persisted.skill.is_none(),
            "clear must be persisted (clear_skill), got {:?}",
            persisted.skill
        );
        let _ = &mut chat;
    }

    /// The set path (pick a skill from the `$` menu) still persists the body.
    #[tokio::test]
    async fn apply_skill_selection_some_persists_body() {
        let store = fresh_store().await;
        let skill_handle = handle(None);
        let mut active_skill = None;
        let mut active_skill_body = None;
        let mut sys_tokens = 0u64;

        apply_skill_selection(
            &Some(("alpha".to_string(), "the alpha body".to_string())),
            &mut active_skill,
            &mut active_skill_body,
            &mut sys_tokens,
            "act",
            std::path::Path::new("/tmp"),
            &skill_handle,
            &store,
            "s",
        )
        .await;

        assert_eq!(active_skill.as_deref(), Some("alpha"));
        assert_eq!(
            skill_handle.lock().unwrap().as_deref(),
            Some("the alpha body")
        );
        let persisted = store.get_session("s").await.unwrap().unwrap();
        assert_eq!(persisted.skill.as_deref(), Some("the alpha body"));
    }
}
