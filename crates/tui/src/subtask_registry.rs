//! Pure registry functions over one-shot remote subtask refs
//! (`opencoder_core::harness::SubtaskRef`). The registry is a plain `Vec`
//! owned by the parent session's harness runtime; every helper here is a
//! total function over that value — no I/O, no async, no globals. The
//! server owns execution truth; these helpers only shape local UI
//! bookkeeping (badges, appended transcript lines, folded token cost).

use opencoder_core::harness::SubtaskRef;

/// Display-safe cap for a fetched result summary. Char-boundary truncation
/// keeps multi-byte text renderable; `bound_summary` applies it.
pub(crate) const SUMMARY_MAX_CHARS: usize = 2048;

/// Register `entry` in a cloned registry, replacing any ref with the same
/// `execution_id` (re-dispatch of a known id overwrites in place).
pub fn register(refs: &[SubtaskRef], entry: SubtaskRef) -> Vec<SubtaskRef> {
    let mut next = refs.to_vec();
    match next
        .iter()
        .position(|item| item.execution_id == entry.execution_id)
    {
        Some(index) => next[index] = entry,
        None => next.push(entry),
    }
    next
}

/// Find a ref by execution id.
pub fn find<'a>(refs: &'a [SubtaskRef], id: &str) -> Option<&'a SubtaskRef> {
    refs.iter().find(|item| item.execution_id == id)
}

/// Whether `status` is a terminal subtask lifecycle state.
pub fn is_terminal(status: &str) -> bool {
    matches!(status, "done" | "error" | "cancelled")
}

/// Execution ids whose subtasks are still in flight (pending/running/…).
pub fn pending_ids(refs: &[SubtaskRef]) -> Vec<String> {
    refs.iter()
        .filter(|item| !is_terminal(&item.status))
        .map(|item| item.execution_id.clone())
        .collect()
}

/// Count of terminal subtasks.
pub fn done_count(refs: &[SubtaskRef]) -> u32 {
    refs.iter().filter(|item| is_terminal(&item.status)).count() as u32
}

/// Total registered subtasks.
pub fn total_count(refs: &[SubtaskRef]) -> u32 {
    refs.len() as u32
}

/// Sum of the folded sub-execution token usage.
pub fn sum_tokens(refs: &[SubtaskRef]) -> u64 {
    refs.iter().map(|item| item.tokens).sum()
}

/// Apply one watcher delivery to a cloned registry. Returns the new registry
/// and the token delta to fold into the parent `[tok cost]` label.
///
/// Idempotency contract:
/// - unknown id → unchanged clone, delta 0;
/// - entry already terminal with the same status → unchanged clone, delta 0
///   (unless the delivery reports grown tokens, in which case only `tokens`
///   is refreshed and the delta is the growth);
/// - any non-terminal → terminal transition always applies the status and
///   the bounded summary.
pub fn apply_update(
    refs: &[SubtaskRef],
    id: &str,
    status: &str,
    summary: Option<&str>,
    tokens: u64,
) -> (Vec<SubtaskRef>, u64) {
    let Some(index) = refs.iter().position(|item| item.execution_id == id) else {
        return (refs.to_vec(), 0);
    };
    let old = &refs[index];
    if is_terminal(&old.status) && old.status == status {
        if tokens > old.tokens {
            let delta = tokens - old.tokens;
            let mut next = refs.to_vec();
            next[index].tokens = tokens;
            return (next, delta);
        }
        return (refs.to_vec(), 0);
    }
    let delta = tokens.saturating_sub(old.tokens);
    let mut next = refs.to_vec();
    next[index].status = status.to_string();
    next[index].result_summary = summary.map(bound_summary);
    next[index].tokens = tokens;
    (next, delta)
}

/// Truncate to [`SUMMARY_MAX_CHARS`] chars (display-safe) appending `…`
/// (U+2026) when truncation happened. No other sanitization.
pub fn bound_summary(text: &str) -> String {
    if text.chars().count() <= SUMMARY_MAX_CHARS {
        return text.to_string();
    }
    let mut bounded: String = text.chars().take(SUMMARY_MAX_CHARS).collect();
    bounded.push('\u{2026}');
    bounded
}

/// Single-line transcript text appended when a subtask finishes:
/// `[subtask <id> · <kind>] Q: <question> / R: <summary-or-(未完成)> / tokens: <n>`.
/// Question and summary collapse to single lines (newlines become spaces);
/// the summary is bounded by [`bound_summary`].
pub fn append_text(entry: &SubtaskRef) -> String {
    let question = single_line(&entry.question);
    let summary = match &entry.result_summary {
        Some(summary) => single_line(&bound_summary(summary)),
        None => "(未完成)".to_string(),
    };
    format!(
        "[subtask {} \u{00b7} {}] Q: {} / R: {} / tokens: {}",
        entry.execution_id, entry.kind, question, summary, entry.tokens
    )
}

fn single_line(text: &str) -> String {
    text.replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, status: &str, tokens: u64) -> SubtaskRef {
        SubtaskRef {
            execution_id: id.into(),
            kind: "agent".into(),
            capability_id: "ops".into(),
            question: "question".into(),
            status: status.into(),
            result_summary: None,
            tokens,
            created_at: 1,
        }
    }

    #[test]
    fn register_appends_and_dedupes_by_execution_id() {
        let refs = vec![entry("agent-a", "pending", 0)];
        let grown = register(&refs, entry("agent-b", "pending", 0));
        assert_eq!(grown.len(), 2);
        let replaced = register(&grown, entry("agent-a", "done", 9));
        assert_eq!(replaced.len(), 2);
        assert_eq!(
            find(&replaced, "agent-a").map(|item| item.status.as_str()),
            Some("done")
        );
        assert_eq!(find(&replaced, "agent-z"), None);
    }

    #[test]
    fn apply_update_is_idempotent_for_repeated_terminal_delivery() {
        let refs = register(&[], entry("agent-a", "done", 100));
        let (next, delta) = apply_update(&refs, "agent-a", "done", Some("same"), 100);
        assert_eq!(next, refs);
        assert_eq!(delta, 0);
        // Grown tokens on a terminal entry refresh only the folded cost.
        let (next, delta) = apply_update(&refs, "agent-a", "done", Some("same"), 150);
        assert_eq!(delta, 50);
        assert_eq!(find(&next, "agent-a").unwrap().tokens, 150);
        assert_eq!(
            find(&next, "agent-a").unwrap().result_summary.as_deref(),
            None
        );
    }

    #[test]
    fn apply_update_transition_returns_delta_and_bounded_summary() {
        let refs = register(&[], entry("agent-a", "running", 40));
        let (next, delta) = apply_update(&refs, "agent-a", "done", Some("answer"), 90);
        assert_eq!(delta, 50);
        let done = find(&next, "agent-a").unwrap();
        assert_eq!(done.status, "done");
        assert_eq!(done.result_summary.as_deref(), Some("answer"));
        assert_eq!(done.tokens, 90);
        // Token regression never folds a negative delta.
        let (_, delta) = apply_update(&next, "agent-a", "done", None, 10);
        assert_eq!(delta, 0);
    }

    #[test]
    fn apply_update_unknown_id_is_a_noop() {
        let refs = register(&[], entry("agent-a", "pending", 1));
        let (next, delta) = apply_update(&refs, "agent-z", "done", Some("x"), 9);
        assert_eq!(next, refs);
        assert_eq!(delta, 0);
    }

    #[test]
    fn counts_and_pending_ids_track_terminal_state() {
        let refs = vec![
            entry("agent-a", "pending", 10),
            entry("agent-b", "done", 20),
            entry("agent-c", "error", 30),
            entry("agent-d", "cancelled", 40),
        ];
        assert_eq!(pending_ids(&refs), vec!["agent-a".to_string()]);
        assert_eq!(done_count(&refs), 3);
        assert_eq!(total_count(&refs), 4);
        assert_eq!(sum_tokens(&refs), 100);
        assert_eq!(done_count(&[]), 0);
        assert_eq!(total_count(&[]), 0);
    }

    #[test]
    fn bound_summary_truncates_on_char_boundaries_only() {
        let short = "a".repeat(SUMMARY_MAX_CHARS);
        assert_eq!(bound_summary(&short).chars().count(), SUMMARY_MAX_CHARS);
        let long = "汉".repeat(SUMMARY_MAX_CHARS + 5);
        let bounded = bound_summary(&long);
        assert_eq!(bounded.chars().count(), SUMMARY_MAX_CHARS + 1);
        assert!(bounded.ends_with('\u{2026}'));
    }

    #[test]
    fn append_text_renders_exact_format_and_collapses_lines() {
        let mut item = entry("agent-1", "done", 77);
        item.kind = "todos".into();
        item.question = "multi\nline question".into();
        item.result_summary = Some("multi\nline summary".into());
        assert_eq!(
            append_text(&item),
            "[subtask agent-1 \u{00b7} todos] Q: multi line question / R: multi line summary / tokens: 77"
        );
        // Missing summary falls back to the unfinished marker.
        item.result_summary = None;
        assert_eq!(
            append_text(&item),
            "[subtask agent-1 \u{00b7} todos] Q: multi line question / R: (未完成) / tokens: 77"
        );
        // Oversized summaries are bounded before collapsing.
        item.result_summary = Some("\n".repeat(SUMMARY_MAX_CHARS + 10));
        let rendered = append_text(&item);
        let summary: String = rendered
            .split("R: ")
            .nth(1)
            .unwrap()
            .split(" / tokens")
            .next()
            .unwrap()
            .to_string();
        assert!(summary.ends_with('\u{2026}'));
        assert!(summary.chars().count() <= SUMMARY_MAX_CHARS + 1);
        assert!(!summary.contains('\n'));
    }
}
