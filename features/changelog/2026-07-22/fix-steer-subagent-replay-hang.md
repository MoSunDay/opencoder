# Fix: steer-subagent replay hang

**Date:** 2026-07-22
**Scope:** `opencoder-session`

## Problem

Clicking **steer** (submitting a redirect) while a subagent was running hung
the steer — it was never consumed. On the drain turn, `run_with_registry`
calls `replay_cancelled_tasks` *before* `claim_steers`. The old implementation
unconditionally replayed every `Cancelled` subagent task to completion, so the
interrupted child was silently re-run, occupying the turn with a tool batch.
Because `claim_steers` only runs once the loop reaches an idle turn, the user's
pending steer was deferred indefinitely — the session appeared frozen on the
steer.

The same replay-to-completion behavior is **correct** for a hard abort /
crash-resume (the user genuinely wants the child finished), so it must not be
removed — only suppressed when the user has explicitly steered.

## Fix

`replay_cancelled_tasks` now distinguishes a user **steer-redirect** from a
hard-abort / crash resume, by checking `store.pending_inputs(id, Delivery::Steer)`:

| Branch | Condition | Behavior |
|--------|-----------|----------|
| Abandon (steer) | pending steers non-empty | Do **not** replay. New `abandon_cancelled_tasks` backfills a terminal error `tool_result` ("cancelled: the user redirected this turn (steer).") for each `tool_use_id` (keeps the transcript well-formed — no dangling ids a provider rejects with HTTP 400) and marks each task `Failed` so it is never replayed again. The drain turn falls through to `claim_steers` and consumes the steer immediately. |
| Replay (default) | no pending steer | Unchanged: replay each `Cancelled` child to completion, guarded by the session `cancel` token (break if already cancelled — the double-Esc case expects no work), then backfill `tool_result` + `complete_subagent_task`. |

| File | Change |
|------|--------|
| `crates/session/src/resume.rs` | `replay_cancelled_tasks`: query `pending_inputs(Delivery::Steer)`; on hit, call new `abandon_cancelled_tasks` and return early. Add `abandon_cancelled_tasks` helper (backfill terminal `ToolResult` + `complete_subagent_task(.., ok=false)`). Restore cancel-token break guard in the replay loop. |

## Tests

`crates/session/tests/resume_replay.rs` — **7 passed**; workspace **790 passed, 0 failed**.

| Test | Asserts |
|------|---------|
| `replay_cancelled_tasks_abandons_when_steer_pending` (new) | pending steer → child **not** replayed (`call_count()==0`); task marked `Failed` (not `Cancelled`); no dangling `tool_use` (terminal `tool_result` backfilled); backfilled result contains "steer" |
| `replay_cancelled_tasks_skips_children_when_cancel_token_fired` (new) | session cancel token already fired → no child replay (`call_count()==0`), task stays `Cancelled` |
| `replay_cancelled_tasks_runs_children_when_token_not_fired` (new) | fresh token → child replayed (`call_count()==1`) |
| `resume_and_replay_*` (4 existing) | `replay_cancel` param threaded through; Running children replayed + backfilled, completed tasks untouched, multiple children folded into one Tool message |
