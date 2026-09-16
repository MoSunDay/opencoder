// fuzzy.js — fuzzy subsequence scorer for the composer command menu.
// Ported 1:1 from the TUI skill picker (crates/tui/src/menu.rs::fuzzy_score)
// so the SPA `@agent` entries rank exactly like the TUI `/agent` picker:
// `query` must be a subsequence of `target`; the score (lower = better)
// rewards compact, consecutive and early matches. Case-insensitive — the
// TUI callers lowercase both sides, this port folds internally instead.

/// Subsequence match score, or `null` when `query` is not a subsequence of
/// `target`. An empty query matches everything with score 0 (same as the
/// TUI: an empty filter lists all rows).
export function fuzzyScore(query, target) {
  const q = Array.from(String(query ?? '').toLowerCase());
  const t = Array.from(String(target ?? '').toLowerCase());
  if (q.length === 0) {
    return 0;
  }
  if (q.length > t.length) {
    return null;
  }
  let qi = 0;
  let score = 0;
  let prevMatch = null;
  for (let ti = 0; ti < t.length && qi < q.length; ti += 1) {
    if (t[ti] === q[qi]) {
      // Consecutive match bonus.
      if (prevMatch !== null && ti === prevMatch + 1) {
        score -= 10;
      }
      // Early match bonus (first few chars of target).
      if (ti < 3) {
        score -= 5;
      }
      score += ti; // earlier = lower score = better
      prevMatch = ti;
      qi += 1;
    }
  }
  return qi === q.length ? score : null;
}
