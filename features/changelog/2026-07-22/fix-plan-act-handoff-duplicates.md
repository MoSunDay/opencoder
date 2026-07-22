# Fix: plan→act handoff duplicates

**Date:** 2026-07-22
**Scope:** `opencoder-tui`, `opencoder-session`

## Problem

Three bugs caused duplicate/orphaned content after plan→act handoff:

1. **Duplicate plan card (P0):** `TranscriptReset` triggered `replay_into_chat`
   which rendered a Plan card from the persisted `handoff_plan`. The
   immediately following `PlanHandoff` event then pushed a second identical
   card — users saw two plan cards.

2. **Stale skill_prompt leak (P1):** After handoff, `skill_prompt` (set during
   plan mode) was not cleared. `run_with_registry` with an empty prompt then
   injected an unwanted synthetic "The active skill is now in effect…"
   message, and the system prompt carried a stale `## Active skill` block.

3. **Orphaned cancelled subagent (P1):** `replay_cancelled_tasks` replayed
   Cancelled subagent tasks from the store even when their parent `ToolUse`
   block had been removed by the handoff collapse. This injected orphan
   `ToolResult` messages whose `tool_use_id` had no matching `ToolUse`,
   risking API 400 errors.

## Fixes

| Fix | File | Change |
|-----|------|--------|
| 1 — Plan card dedup | `crates/tui/src/chat.rs` | `PlanHandoff` handler early-returns if `self.blocks` already contains a `ChatBlock::Plan` |
| 2 — Clear skill_prompt | `crates/tui/src/worker.rs` | `sess.set_skill(None)` added after the handoff block in `SwitchAndStart` |
| 3 — Filter orphan tasks | `crates/session/src/resume.rs` | `replay_cancelled_tasks` filter: when `handoff_seq.is_some()`, require the task's `tool_use_id` to exist as a `ToolUse` in `session.messages`; when `handoff_seq.is_none()`, preserve original behavior (allow all Cancelled tasks) |

## Tests

| Test | File | Asserts |
|------|------|---------|
| `plan_handoff_skips_duplicate_plan_card` | `crates/tui/tests/plan_card_dedup.rs` (new file) | Two PlanHandoff events → only 1 Plan block |
| `switch_and_start_clears_skill_prompt` | `crates/tui/tests/plan_act_handoff.rs` | After SwitchAndStart, skill_prompt is None; act LLM request has no skill trigger message |
| `handoff_skips_orphaned_cancelled_subagent` | `crates/session/tests/plan_handoff.rs` | After handoff, cancelled subagent task whose ToolUse was removed → no Tool message injected |
