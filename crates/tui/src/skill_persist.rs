//! Persisting the active skill so it survives resume/restart.
//!
//! When a `{$skill}` token is submitted inline — most importantly when it is
//! **queued** via Tab while a turn is running — `resolve_and_warn` activates the
//! skill in-memory (`SessionState::skill_prompt`) but, unlike the skill-menu
//! (`SetSkill`) path, it never wrote the skill body to the store. A combined
//! submission like `{$skill} fix the bug` therefore queued/persisted only the
//! clean task text, so on resume the queued task ran **without** the skill.
//!
//! [`persist_skill`] mirrors the `SetSkill` path: a best-effort
//! `update_session(skill=…)` whenever the skill body changed.

use std::sync::{Arc, Mutex};

use opencoder_core::message::now_ms;
use opencoder_store::{SessionPatch, Store};

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

    /// Mirrors a combined `{$skill} do X` submit: the token resolves and sets
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

    /// Regression for the reported bug: a **queued** `{$skill} fix the bug`
    /// submission. `extract_skill_tokens` strips the token and yields the clean
    /// task text; the resolved skill body lands in `skill_prompt`. Previously
    /// that body was in-memory only, so on resume the queued task ran with no
    /// skill. Now `persist_skill` carries it into the store row.
    #[tokio::test]
    async fn persist_skill_survives_combined_queued_skill_submission() {
        use opencoder_core::extract_skill_tokens;

        let store = fresh_store().await;
        let raw = "{$repo-memory} fix the bug";
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
            store.get_session("s").await.unwrap().unwrap().skill.as_deref(),
            Some("the repo-memory skill body"),
            "queued combined skill+text must persist the skill for resume"
        );
    }

    /// Switching skills mid-session (`{$a}` then later `{$b}`) persists the new
    /// body, not the old one.
    #[tokio::test]
    async fn persist_skill_updates_when_skill_changes() {
        let store = fresh_store().await;
        let skill_handle = handle(Some("body-b"));
        persist_skill(&store, "s", &Some("body-a".into()), &skill_handle).await;

        let persisted = store.get_session("s").await.unwrap().unwrap();
        assert_eq!(persisted.skill.as_deref(), Some("body-b"));
    }
}
