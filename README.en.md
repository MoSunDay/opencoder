<p align="center">
  <img src="logo/logo.png" alt="OpenCoder" width="220" />
</p>

<h1 align="center">OpenCoder</h1>

<p align="center">
  A Rust-native coding agent built from scratch · single binary · swappable storage & LLM backends<br/>
  A high-performance, minimal coding agent written in Rust.
</p>

<p align="center">
  <img alt="version" src="https://img.shields.io/badge/version-0.1.0-blue" />
  <img alt="rust" src="https://img.shields.io/badge/Rust-2021-orange?logo=rust" />
  <img alt="license" src="https://img.shields.io/badge/license-MIT-green" />
  <img alt="platform" src="https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey" />
  <img alt="status" src="https://img.shields.io/badge/status-active%20development-yellow" />
</p>

<p align="center">
  <a href="./README.md">简体中文</a> · <strong>English</strong>
</p>

---

> ## ⚠️ Risk Notice: This project trusts the model 100% and enforces NO permission checks
>
> **OpenCoder has no permission interception and no sandbox.** Once started, the model is granted **exactly the same** privileges as your OS user: it can read, write, modify, and delete any file reachable by your account, and execute any shell command (including `rm`, `git push --force`, writing to databases, network downloads, etc.) **without any secondary confirmation prompt.**
>
> In other words, **running OpenCoder is equivalent to handing full control of this machine to the model.**
>
> Before using it, please:
>
> - 📌 **Assess the risk first**: only use it if you accept that "the model may delete/overwrite any file and run any command."
> - 💾 **Back up important data**: back up or version-control the code, data, and configs you care about, and make sure the backup lives outside any directory this process can write to.
> - 🔐 **Isolate the environment**: run it inside a container, VM, or a dedicated low-privilege account. Never run it directly on a production server or in a working directory that holds sensitive credentials (API keys / private keys / DB connection strings).
> - 👀 **Stay attentive**: in headless `run` and `serve` modes commands execute automatically; keep the working directory and privilege scope within your control.
>
> **If you do not trust the model, please do not use this project without adequate backups and isolation.** The author is not liable for any data loss or damage caused by the model's behavior.

---

OpenCoder is a fully independent, Rust-native coding agent implemented from scratch. It ships a single binary `opencoder` that offers **four working modes**: an **interactive TUI**, **headless one-shot execution**, a **centralized HTTP/SSE server**, and a **remote thin client**. All upper-layer logic depends on only two abstraction seams — `Arc<dyn Store>` and `Arc<dyn ChatStream>` — so both the persistence layer (libsql) and the LLM backend (OpenAI-compatible) are swappable.

## ✨ Features

- **🧠 Multi-mode runtime** — TUI interaction, headless `run`, `server` (HTTP/JSON + SSE), and `client` remote thin frontend; all four entry points share the same session runtime.
- **🔄 Session resume & fork** — `--session <id>` / `--continue` / `--fork` rebuild history from libsql across processes; titles are generated asynchronously by the small model.
- **📦 Session binary export/import** — `session export/import` carries the full subagent tree in a `.opencoder` binary (`OPENCODR` magic) for migration; idempotent and never exports Config (API key safe).
- **🛠️ Subagent scheduling** — two subagent kinds, `explore` (read-only investigation) and `build` (implementation execution), with DB-tracked lifecycles and collapsible views.
- **📋 Plan / Act dual mode** — Plan mode is read-only (bash writes are blocked by `bash_guard`); switching to Act clears the context and keeps only the final plan.
- **🗜️ Auto-compaction** — token-estimate-driven context compression; `compaction.{auto,context_threshold,reserved,tail_turns,buffer}` is fully configurable.
- **🎮 Steer / follow-up two-phase delivery** — inject steer instantly at turn boundaries while running; consume exactly one queued follow-up when idle.
- **🌐 Lossless event replay** — SSE event schema v2 migration; `SessionEvent` is the single source of truth, and replay fully rebuilds tool blocks.
- **⚡ High performance** — cold start ~6 ms, binary 9.3 MB (thin-LTO + strip); libsql WAL concurrent read/write, appending 1k messages in 30 ms.

## 🚀 Quick Start

### Installation

Build from source (requires a Rust toolchain):

```bash
git clone https://github.com/MoSunDay/opencoder.git
cd opencoder
cargo build --release
# Binary located at target/release/opencoder
```

Or use the install script:

```bash
curl -fsSL https://raw.githubusercontent.com/MoSunDay/opencoder/main/scripts/install.sh | bash
```

### Configuration

Place an `opencoder.json` in the project root or `~/.opencode/` (env vars and CLI flags take higher precedence):

```jsonc
{
  "provider": "openai",
  "model": "glm-4.6",
  "small_model": "glm-4.5-air",
  "context_limit": 128000,
  "max_tokens": 8000,
  "reasoning_effort": "medium",
  "compaction": { "auto": true, "context_threshold": 0.8 }
}
```

### Three usage modes

```bash
# 1) Interactive TUI
opencoder

# 2) Headless one-shot run, output to stdout
opencoder run "Implement an LRU cache in Rust with tests"

# 3) Start the server (centralized storage + LLM gateway + SSE); another machine connects via client
opencoder server --host 0.0.0.0 --port 8080
opencoder client --remote http://127.0.0.1:8080 "Summarize this repo's architecture"
```

## 🧱 Architecture

OpenCoder is a Cargo workspace composed of 8 crates with strictly layered dependencies:

| Crate | Responsibility |
| --- | --- |
| `core` | Shared types and `Config` (model / compaction / context window / small_model, all configurable) |
| `llm` | OpenAI-compatible streaming client + `ChatStream` trait + `MockChatClient` + token estimator |
| `store` | `Store` trait + libsql implementation (WAL, embedded), the sole outlet for all persistence |
| `session` | Runtime core: drain main loop, tool registration, subagent scheduling, plan bash guard, compaction, resume |
| `tui` | ratatui interactive UI (3-pane layout, subagent folding, steer/follow-up, plan/act switch) |
| `web` | axum HTTP + SSE session management (prompt admit / event stream / runtime switch / interrupt) |
| `client` | Remote thin client: submits prompts and replays the stream; stores nothing locally, calls no LLM |
| `cli` | clap frontend + headless runtime (run / tui / server / client / config / models / session) |

**Key abstractions:**

- **`Store` trait** (`crates/store/src/store.rs`) — unified CRUD for sessions / messages / inputs / events / subagent_tasks; the only seam for swapping SQLite implementations.
- **`ChatStream` trait** (`crates/llm/src/stream.rs`) — implemented by both `ChatClient` (real) and `MockChatClient`, enabling zero-token deterministic testing of the session runtime.
- **drain semantics** (`crates/session/src/runner.rs::run_loop`) — promotes steer at every turn boundary; when idle it consumes exactly one queued item; a doom-loop guard (`DOOM_THRESHOLD=3`) breaks consecutive empty turns.

## 📖 Command Reference

```
opencoder [OPTIONS] [PROMPT]...        # Default: enter the TUI
opencoder run <PROMPT>                  # Headless one-shot run
opencoder tui                           # Explicitly launch the TUI
opencoder server [--host] [--port]      # Server (alias: serve)
opencoder client --remote <URL> <PROMPT># Remote thin client
opencoder config [show]                 # Inspect merged config
opencoder models                        # List known models
opencoder session <list|show|delete>    # Session management (show --json is a deep-inspection view)

Global options:
  -m, --model <MODEL>          Specify the main model
      --small-model <MODEL>    Specify the small model (title generation, etc.)
      --agent <explore|build>  Specify the agent type
      --workdir <PATH>         Working directory
  -s, --session <ID>           Resume a specific session
      --continue               Resume the most recent session in the current workdir
      --fork                   Copy before resuming; the original session is left unchanged
  -v, --verbose                Verbose logging
```

## ⚡ Performance

| Metric | Measured | Target |
| --- | --- | --- |
| Cold start (`--help`) | **~6 ms** | < 50 ms |
| Binary size | **9.3 MB** | — |
| Append 1000 messages (transaction) | 30.5 ms → **0.031 ms/msg** | < 2 ms/msg |
| Load 1000 messages | **2.4 ms** | < 50 ms |
| List 200 sessions | **0.95 ms** | < 100 ms |
| Full deterministic test suite | **~3 s / 384 tests** | — |

See [`docs/perf.md`](docs/perf.md) for details.

## 📊 Comparison with opencode

OpenCoder and [sst/opencode](https://github.com/sst/opencode) (a TypeScript / Node SEA implementation) target the same "coding agent" use case, but their runtime baseline overhead differs significantly. The table below is a **measured runtime-baseline comparison** (not an end-to-end task benchmark), reproducible on the same machine under the same load.

**Test environment:** Intel Xeon E5-2673 v3 @ 2.40GHz · 24 cores · Ubuntu 22.04 (Linux 6.8) · 2026-07-19

**Methodology:** workload = `--help` (the minimal workload, reflecting the runtime's own overhead and excluding LLM RTT); peak memory via *Maximum resident set size* from `/usr/bin/time -v`; 5 samples each, median reported.

| Metric | opencode `1.17.8` | opencoder `0.1.0` | Difference |
| --- | --- | --- | --- |
| Runtime | Node SEA (V8, single executable app) | Rust-native single binary | — |
| Binary size | **159 MiB** (166,885,504 B) | **11.1 MiB** (11,644,304 B) | opencode is **14.3×** larger |
| Cold-start peak RSS | **~197 MiB** (195–199 MB) | **~5.4 MiB** (5.2–5.6 MB) | opencode is **35×** higher |
| Cold-start time | **~0.78 s** | **~6 ms** | opencode is **~125×** slower |
| Cold-start token overhead | **~13k tok** | **~0.4k tok** | opencode **~32×** higher |
| Protocol | OpenAI-compatible + ACP + MCP | OpenAI-compatible + subagents (explore/build) | — |
| Session storage | SQLite (WAL) + Drizzle ORM | libsql (SQLite-compatible, WAL) + hand-written `Store` trait | Both SQLite; see [storage comparison](#-storage) below |

> **Cold-start token overhead** methodology: take each agent's first request body sent to the LLM, count tokens in the `system` message + the `tools` array (the fixed first-turn cost, incurred before any user input) with the `cl100k_base` BPE. Measured independently from the other `--help` runtime-baseline rows above. This is a fixed cost re-paid on every conversation turn.

### 🗄️ Storage

Both are **SQLite under the hood**, persisting as standard `.db` + `.db-wal` + `.db-shm` (WAL mode); the differences are at the driver layer and table model:

| Dimension | opencode `1.17.8` | opencoder `0.1.0` |
| --- | --- | --- |
| Database | SQLite (embedded in bun) | libsql `0.9.30` (SQLite-compatible, `core` + `libsql-sys`) |
| Access layer | Drizzle ORM (JS, runtime migrations via `migration` table) | Hand-written SQL + `Store` trait (`crates/store/src/store.rs`) |
| Driver language | JS/V8 ↔ SQLite C ABI | Rust ↔ libsql C ABI (zero-copy binding) |
| WAL PRAGMA | `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`, `cache_size=-64000`, `foreign_keys=ON` | `journal_mode=WAL` (same concurrency model) |
| Tables (core) | `session` / `message` / `step` / `agent` / `tool_use` / `snapshot` / `checkpoint` / `event` / `subagent` | `sessions` / `messages` / `session_inputs` / `session_events` / `subagent_tasks` / `schema_version` |
| Migration | Drizzle migration (runtime diff against `migration` table + `meta/_journal.json`) | Single `schema_version` value + idempotent `CREATE TABLE IF NOT EXISTS` |
| DB file naming | `opencode.db` (varies by channel, e.g. `opencode-dev.db`) | Determined by workdir (one DB per workdir) |

> Both share an equivalent WAL concurrency model (many readers / one writer; reads never block writes). Storage throughput differences mainly come from the driver layer — opencode goes through Drizzle ORM + V8 cross-language hops, while opencoder talks directly to the libsql C binding with no ORM translation layer.

> Note: end-to-end task performance is LLM-dominated (first-token latency is the provider RTT); under the same model both projects are essentially neck-and-neck in actual task throughput. The comparison above isolates the **runtime baseline** — the cost the runtime itself imposes regardless of the LLM.

Using "implement a terminal Snake game in Rust + crossterm" as a control task, both invoked the same way:

```bash
# opencode (Node SEA)
opencode run --model zhipuai-coding-plan/glm-5.2 "Implement terminal Snake..."

# opencoder (Rust native)
opencoder run "Implement terminal Snake..."
```

**Measured comparison (2026-07-19, same machine, same model glm-5.2, isolated workdirs, 0.5 s sampling throughout):**
**Scope: only "non-compilation periods" are counted — the agent main process itself, with no cargo/rustc child process running (samples where `npids<=1`). Fully symmetric on both sides, completely excluding compilation contributions.**

| Metric | opencode `1.17.8` | opencoder `0.1.0` | Difference |
| --- | --- | --- | --- |
| Task completion time (wall) | 172.3 s | 112.2 s | LLM sampling noise is large; reference only |
| Non-compilation sample share | 313 / 321 (97.5%) | 199 / 209 (95.2%) | Compilation share is small for both |
| **Avg CPU utilization** | **55.3%** (sustained high) | **0.13%** (event-driven, idle while waiting on LLM) | opencode **~425×** higher |
| CPU median (p50) | **44.6%** (≥44% half the time) | **0.0%** (fully 0 half the time) | Fundamental difference |
| CPU p95 | 115.2% | 1.9% | opencode **~61×** higher |
| CPU peak | 2631.6% (V8 GC/JIT, transient ≈26 cores) | 3.7% | — |
| **Agent process RSS avg** | **451.7 MB** | **11.8 MB** | opencode **~38×** higher |
| Agent process RSS peak | 557.5 MB | 12.1 MB | opencode **~47×** higher |
| Result | compiles, 242 lines | compiles, 351 lines | — |

> **Key: opencode's high CPU is unrelated to compilation (self-proven by the data).** The opencode 2631.6% CPU-peak sample was verified to have `npids=1` (no cargo child running at that instant) — it is a pure V8 GC/JIT burst. Moreover, after stripping all compilation periods, opencode's average CPU did not drop but rose slightly (full-run 54.4% → non-compilation 55.3%). In other words, during "waiting for LLM reply" / "orchestrating tools" periods that have nothing to do with compilation, opencode's own V8 runtime still sustains ~55% continuous high CPU. opencoder is the opposite — non-compilation CPU averages 0.13%, median 0; the agent is essentially silent while waiting.

> **Scope & credibility notes:**
> - All numbers in the table come from **non-compilation-period samples** (`npids<=1`, agent main process with no cargo/rustc child), fully symmetric on both sides. Compilation periods (opencode 2.5%, opencoder 4.8%) are all excluded.
> - **Wall-time is affected by LLM sampling randomness**: this run opencode 172.3 s / opencoder 112.2 s; the previous run was 125.7 s / 79.0 s in the opposite direction, so treat it as reference only, **not a steady-state conclusion**; CPU utilization and RSS are the stable differences in the runtime baseline.
> - opencode `run` bundles a V8 runtime and sustains resident high CPU/RSS; opencoder compiles to a native async runtime and drops CPU to ~0 while waiting for replies.
> - opencode also has a resident server daemon (≈285 MB) not counted here; in long-running server mode its resident overhead is even higher.
> - Raw CSV evidence (including the six `root_*` / `tree_*` / `npids` columns) is at [`docs/bench/opencode-vs-opencoder-2026-07-19/`](docs/bench/opencode-vs-opencoder-2026-07-19/).

---

## 🧪 Development & Testing

This project strictly follows the development rules under [`rules/`](rules/): every business feature must have corresponding tests, and a full regression run with a changelog + test manifest is required before each iteration ends.

```bash
# Unit + integration tests (deterministic, zero network)
cargo test --workspace

# Real-model end-to-end contract tests (~3–5 min, requires an API key)
scripts/e2e-glm.sh
```

See [`rules/03-test-pyramid.md`](rules/03-test-pyramid.md) for the test-layering spec.

## 📁 Project Structure

```
opencoder/
├── crates/
│   ├── core/      # Shared types & Config
│   ├── llm/       # LLM client + ChatStream trait
│   ├── store/     # Store trait + libsql implementation
│   ├── session/   # Session runtime core
│   ├── tui/       # ratatui interactive UI
│   ├── web/       # axum HTTP + SSE
│   ├── client/    # Remote thin client
│   └── cli/       # clap frontend + headless runtime
├── docs/          # Performance profiles & other docs
├── features/      # Capability map + date-archived changelogs
├── rules/         # Development rules (testing / regression / layering)
├── scripts/       # Install scripts, e2e tests
├── logo/          # Project logo
└── src/main.rs    # Binary entry point
```

## 🙏 Acknowledgements

This project's optional capabilities stand on the shoulders of these excellent projects:

| Project | How it contributes |
| --- | --- |
| [obscura](https://github.com/h4ckf0r0day/obscura) | Underlying browser engine dependency (feature-gated `browser`): headless rendering based on deno_core / V8, driving the JS execution and anti-bot handling behind `web_fetch` / `web_search`. |
| [agent-browser](https://github.com/h4ckf0r0day/agent-browser) | Content-extraction algorithm reference for `crates/session/src/tools/web_read.rs`: markdown Accept negotiation, `.md` fallback, `llms.txt` / `llms-full.txt` ancestor crawling, readable main-text extraction. |
| [cua](https://github.com/h4ckf0r0day/cua) | Computer-use loop reference for `crates/core/src/computer_use.rs`: the perceive → act loop is distilled into the native `ComputerUseExecutor` trait + `ComputerUseLoop`. |

All three are independent implementations: obscura is wired in as a rev-pinned git dependency; the algorithms/ideas of agent-browser and cua are ported / distilled into pure Rust (no source dependency on them).

## 📄 License

[MIT](LICENSE)
