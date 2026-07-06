# OpenCoder Performance Profile

Measured on the Rust implementation vs the opencode TypeScript reference.
Hardware: linux/x86_64 dev container. Release build (`lto="thin"`, `codegen-units=1`, `strip=true`).

## 1. Cold startup (`--help`)

| Binary | median | range |
|---|---|---|
| **opencoder (Rust, release)** | **~6 ms** | 5–8 ms |
| opencode (Bun, TS reference) | ~1489 ms | 1460–3610 ms |

Rust is ~250× faster to first response. Target was < 50 ms — met with 8× headroom.

Binary size: **9.3 MB** (stripped, thin-LTO).

## 2. Store latency (libsql, WAL) — `crates/store/tests/store_perf.rs`

Threshold-asserted contracts (release, single-shot):

| Operation | Measured | Target | Margin |
|---|---|---|---|
| append 1000 messages (transactional) | 30.5 ms total → **0.031 ms/append** | < 2 ms each | 64× |
| load 1000 messages | **2.4 ms** | < 50 ms | 20× |
| list 200 sessions (with preview subquery) | **0.95 ms** | < 100 ms | 100× |

The append path is one transaction for the whole batch (all-or-nothing); per-message
cost is dominated by the WAL commit amortized across the batch.

## 3. Concurrent read/write (WAL) — `store/tests/store_integration.rs`

8 concurrent readers + 1 writer, 100 seeded + 100 appended messages:
all readers observe consistent snapshots, writer completes, final count exact.
No read-blocking observed under WAL.

## 4. Test suite cost

Full workspace unit + integration suite (`cargo test --workspace`): **~2 s** for 227
deterministic tests (MockChatClient-driven, zero network). The glm5.2 e2e
(`scripts/e2e-glm.sh`) is a separate ~3–5 min gate that hits the real provider.

## 5. Notes & methodology

- Startup measured via `date +%s%N` wall-clock around `opencoder --help` (5 samples).
- Store timings from `cargo test --release -p opencode-store --test store_perf -- --nocapture`.
- First-token latency is provider-bound (glm5.2 RTT); the Rust client adds no buffering
  beyond the SSE decoder, so it tracks the model RTT within ~1 ms locally.
- Token estimation (the compaction trigger) is O(n) over message text — negligible vs
  the LLM call it gates.
