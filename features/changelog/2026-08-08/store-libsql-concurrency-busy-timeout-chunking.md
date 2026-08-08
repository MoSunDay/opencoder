# store: libsql concurrency hardening (busy_timeout + batch chunking + WAL checkpoint)

**Date:** 2026-08-08
**Crate:** `opencoder-store`
**Baseline tests:** 72 (pre-existing) → **80 passed** (7 new regression tests; +1 from sibling requirement v8 migration test)

## Problem

Concurrent writers (parallel subagent sessions sharing one `Arc<dyn Store>`)
could hit `SQLITE_BUSY` under contention. The previous `busy_timeout=5000`
(5 s) was insufficient for heavy multi-writer workloads, and unbounded batch
INSERT transactions caused WAL bloat and prolonged lock holds.

## Changes

### `crates/store/src/libsql_store/schema.rs`
- **`busy_timeout` raised to 30000 ms** (30 s) and moved to first position in
  PRAGMAS so it is active before any lock-acquiring pragma executes.
- **Added `PRAGMA wal_autocheckpoint=1000`** to trigger passive WAL
  checkpoints at 1000 pages (≈4 MB), preventing unbounded WAL growth.
- **Added `checkpoint_wal()`** function — best-effort `PRAGMA
  wal_checkpoint(PASSIVE)` called on open to merge any leftover WAL.

### `crates/store/src/libsql_store/mod.rs`
- Both `open()` and `open_memory()` now call `conn.busy_timeout(Duration::from_secs(30))`
  as a belt-and-suspenders complement to the PRAGMA.
- `open()` performs a best-effort `schema::checkpoint_wal()` after bootstrap.

### `crates/store/src/libsql_store/messages.rs`
- **`BATCH_CHUNK = 200`** constant bounds transaction size for batch INSERTs.
- `append_many` refactored to chunk messages into batches of 200, each in its
  own transaction (`append_chunk_in_tx`).
- `import` refactored similarly (`import_chunk_in_tx`).
- Smaller transactions = shorter lock holds = less contention.

## Test Coverage

`crates/store/tests/store_concurrency.rs` (7 tests):

| Test | Validates |
|------|-----------|
| `pragma_busy_timeout_is_30000` | PRAGMA value = 30000 |
| `pragma_wal_autocheckpoint_is_1000` | PRAGMA value = 1000 |
| `append_many_chunks_large_batch` | 250 msgs (2 chunks: 200+50) all persisted |
| `append_many_empty_returns_empty` | Empty batch → empty vec, no error |
| `append_many_exact_chunk_boundary` | Exactly 200 (one full chunk) |
| `import_chunks_large_batch` | 450 msgs via import path (3 chunks) |
| `concurrent_writers_no_deadlock` | 4 concurrent writers, no deadlock |

```
cargo test -p opencoder-store → 80 passed; 0 failed
cargo clippy -p opencoder-store --all-targets -- -D warnings → clean
cargo build -p opencoder-store → success
```
