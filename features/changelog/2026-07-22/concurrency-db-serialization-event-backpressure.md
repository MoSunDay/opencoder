# Fix: DB-op serialization + event backpressure (multi-subagent deadlock)

**Date:** 2026-07-22
**Scope:** `opencoder-store`, `opencoder-tui`

## Problem

With multiple subagents running concurrently, each spawning its own event
flusher (a background task that persists parent-session events), the store and
the UI channel could both fall over:

1. **Store contention / worker starvation (P0):** libsql 0.9.x runs sync SQLite
   FFI directly on tokio worker threads with no serialization. Concurrent store
   operations (multi-subagent flushers + the run_loop) contended on SQLite's
   internal mutex, producing sporadic "cannot start a transaction" errors and
   starving the runtime — manifesting as a deadlock/hang when several
   subagents were dispatched in one turn.

2. **UI channel backpressure loss (P1):** Every `SessionEvent` was pushed to the
   UI mpsc channel via a raw `try_send`, silently dropping on a full channel.
   During a token burst the bounded channel could fill with streaming deltas,
   after which lifecycle events (TurnDone, Error) were also dropped — the UI
   froze or lost the turn boundary.

## Fixes

| Fix | File | Change |
|-----|------|--------|
| 1 — DB-op serialization | `crates/store/src/libsql_store/mod.rs` | Add a `tokio::sync::Mutex<()>` (`db_lock`) to `LibsqlStore`; acquire a guard at the top of all 26 async `Store` methods. An async Mutex yields on contention (never blocks a worker thread) while ensuring at most one worker touches SQLite FFI at a time. |
| 2 — Backpressure-aware delta drop | `crates/tui/src/worker.rs` | `forward_event()` replaces 12 raw `try_send` sites. Recoverable streaming deltas (`TextDelta`/`ReasoningDelta`/`SubagentChild` wrapping those) are dropped when free capacity ≤ `DELTA_MIN_CAPACITY` (64); final text is always rebuilt from the store on `TurnDone`, so no data is lost. Lifecycle events always get a slot via `try_send`. |

## Tests

| Test | File | Asserts |
|------|------|---------|
| `concurrent_store_ops_serialized` | `crates/store/tests/concurrent_serialized.rs` (new file) | 16 tasks × 25 iters interleave `append_events` (transaction), `append_message`, `claim_next_queue` (IMMEDIATE txn) on one shared `LibsqlStore`; asserts zero errors + full data integrity (message/event counts). |
| `multi_subagent_no_deadlock` | `crates/session/tests/subagent.rs` | Dispatches 3 concurrent subagents (each with its own flusher) in one turn; asserts all 3 complete and return without deadlock. |

### Regression gate (current run)

```
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings (Finished 14.50s)
cargo build  --workspace                                 → clean (Finished)
cargo test   --workspace                                 → 784 passed; 0 failed
```
