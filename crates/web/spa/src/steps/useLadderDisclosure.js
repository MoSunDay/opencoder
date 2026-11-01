// useLadderDisclosure.js — LIVE step-ladder disclosure policy for the SPA
// (counterpart of the TUI's chat_step_live.rs): WHICH ladder levels are
// open while a steps turn runs, and who wins when the user clicks.
//
//   * AUTO — while `running` is true the running position auto-opens: the
//     turn row (L0 `steps`) plus the LAST step (L1 `step:N`) and that
//     step's calls aggregate (L2 `calls:N`), so reasoning deltas and call
//     rows stream in place instead of hiding behind the collapsed
//     `▸ N Steps` row. A step born while still running moves the auto slot
//     (`step:1` → `step:2`), so the finished step above folds back on its
//     own. When running retires (Say begins / the turn settles) the auto
//     keys empty out and every auto-managed level collapses back to the
//     `N Steps + Say` summary row.
//   * USER PRIORITY — clicking any level records an explicit open/closed
//     override for that key. Effective state is `override ?? auto`, so a
//     user-closed level never reopens from streaming and a user-opened old
//     step stays open when the auto slot moves on. Overrides persist for
//     the component's lifetime. Ctrl+L / ⤒ 收起 remounts the bubbles (the
//     epoch key in transcript.jsx), so overrides reset; if the turn is
//     still running the auto-open re-applies afterwards (accepted
//     divergence from the TUI's per-turn `user_touched` pinning).
//
// Per-call rows (L3 `tool`) stay uncontrolled: their disclosure is never
// auto-managed (stepsBlock.jsx owns them). Expanding one notifies
// stepsBlock, which pins the enclosing `step:N` + `calls:N` open through
// setOpen overrides (TUI Call-row `user_touched` parity) so the drilled
// call survives the settle.

import { useCallback, useMemo, useState } from 'react';

/// The auto-managed open keys while `running`: the turn row, the LAST
/// (running) step and that step's calls aggregate. Idle (or a turn with
/// no steps yet) opens nothing.
export function autoOpenKeys(running, stepCount) {
  return running && stepCount > 0
    ? ['steps', 'step:' + stepCount, 'calls:' + stepCount]
    : [];
}

/// Disclosure controller for ONE steps turn. StepsContent creates it once
/// and hands it down (StepCollapse consumes the same map for its own
/// `step:N` / `calls:N` keys) — this hook is the ONLY place disclosure
/// decisions are made.
export function useLadderDisclosure(running, stepCount) {
  // key → explicit user open/closed; immutable replace on every click.
  const [overrides, setOverrides] = useState(() => new Map());
  const autoKeys = useMemo(() => autoOpenKeys(running, stepCount), [running, stepCount]);
  const autoSet = useMemo(() => new Set(autoKeys), [autoKeys]);
  // A user override always wins over the auto position.
  const isOpen = useCallback(
    (key) => (overrides.has(key) ? overrides.get(key) : autoSet.has(key)),
    [overrides, autoSet],
  );
  const setOpen = useCallback((key, open) => {
    setOverrides((prev) => {
      const next = new Map(prev);
      next.set(key, open);
      return next;
    });
  }, []);
  return { isOpen, setOpen, autoKeys };
}
