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

OpenCoder is a fully independent, Rust-native coding agent implemented from scratch. It ships a single binary `opencoder` that offers **four working modes**: an **interactive TUI**, **headless one-shot execution**, a **centralized HTTP/SSE server**, and a **remote thin client**. All upper-layer logic depends on only two abstraction seams — `Arc<dyn Store>` and `Arc<dyn ChatStream>` — so both the persistence layer (RocksDB, `rdb`) and the LLM backend (OpenAI-compatible) are swappable.

## ✨ Features

- **🧠 Multi-mode runtime** — Local TUI and headless `run` share the session runtime; dedicated `opencoder-server` and `opencoder-agent` binaries provide Fleet control and node execution.
- **🔄 Session resume & fork** — `--session <id>` / `--continue` / `--fork` rebuild history from the local `rdb` store across processes; titles are generated asynchronously by the small model.
- **📦 Session binary export/import** — `session export/import` carries the full subagent tree in a `.opencoder` binary (`OPENCODR` magic) for migration; idempotent and never exports Config (API key safe).
- **🛠️ Subagent scheduling** — two subagent kinds, `explore` (read-only investigation) and `build` (implementation execution), with DB-tracked lifecycles and collapsible views.
- **📋 Plan / Act dual mode** — Plan mode is read-only (bash writes are blocked by `bash_guard`); switching to Act clears the context and keeps only the final plan.
- **🗜️ Auto-compaction** — token-estimate-driven context compression; `compaction.{auto,context_threshold,reserved,tail_turns,buffer}` is fully configurable.
- **🎮 Steer / follow-up two-phase delivery** — inject steer instantly at turn boundaries while running; consume exactly one queued follow-up when idle.
- **🌐 Lossless event replay** — SSE event schema v2 migration; `SessionEvent` is the single source of truth, and replay fully rebuilds tool blocks.
- **⚡ High performance** — cold start ~9 ms; storage commits through atomic write batches (`WriteBatch`); rdb single-shot measured: append 1000 msgs 5.6 ms (0.006 ms/msg), load 1000 msgs 1.7 ms, list 200 sessions 4.1 ms, threshold contracts in [store_perf.rs](crates/store/tests/store_perf.rs); binary ~42 MB (thin-LTO + strip, RocksDB statically linked).

## 🚀 Quick Start

Native Windows 11 x64 support covers the TUI and operator node with stable PowerShell 7.4 or newer in the 7.x series. See [Windows installation and scope](docs/windows.md).

> 👉 New to OpenCoder? See the [**Quick Start guide**](docs/quickstart.en.md) — TUI keys, browsing/editing files with notepad, attaching notes to tasks, and more.

### Installation

Build from source (requires a Rust toolchain):

```bash
git clone https://github.com/MoSunDay/opencoder.git
cd opencoder
cargo build --release
# Binary located at target/release/opencoder
```

Platform prerequisites (building from source):

- **Linux**: `cargo build --release` works out of the box with the full feature set (including the `opencoder-agent` node).
- **macOS**: builds and runs `opencoder` (CLI/TUI) and `opencoder-server`; the storage engine is pure Rust and needs no libclang or cmake. On macOS, session child processes run as direct children (no Linux pidfd supervision), and the `opencoder-agent` node (runc sandbox, process supervision) is Linux-only.

Or use the install script:

```bash
curl -fsSL https://raw.githubusercontent.com/MoSunDay/opencoder/main/scripts/install.sh | bash
```

### Configuration

On the first interactive launch, OpenCoder creates `~/.opencoder/config.json` and opens an in-TUI model setup form when the effective configuration is not yet usable. After the provider, model, endpoint, and API key pass local validation, the same TUI continues directly to the task composer. Existing usable project or environment configuration is honored without prompting.

Place an `opencoder.json` in the project root or `~/.opencoder/` (env vars and CLI flags take higher precedence):

```jsonc
{
  "model": "openai/glm-4.6",
  "small_model": "openai/glm-4.5-air",
  "context_limit": 128000,
  "max_tokens": 8000,
  "reasoning_effort": "medium",
  "compaction": { "auto": true, "context_threshold": 0.8 },
  "providers": {
    "deepseek": {
      "base_url": "https://api.deepseek.com/v1",
      "api_key": "{DEEPSEEK_API_KEY}",
      "model": "deepseek-chat",
      "headers": [{ "name": "X-Region", "value": "eu" }]
    }
  }
}
```

`model` uses the format `"{provider}/{model_id}"`; the provider name matches a key in the `providers` map (falls back to the default `provider` if unmatched). `api_key` and header `value` support `{ENV_VAR}` indirection.

### Three usage modes

```bash
# 1) Interactive TUI
opencoder

# 2) Headless one-shot run, output to stdout
opencoder run "Implement an LRU cache in Rust with tests"

# 3) Start the Fleet server and connect an execution node from another machine
opencoder-server --host 0.0.0.0 --port 8080 --token-file /secure/path/token
opencoder-agent --remote http://SERVER:8080 --name worker-1 --token-file /secure/path/token
```

## 🧱 Architecture

OpenCoder is a Cargo workspace with layered dependencies. Its main crates include:

| Crate | Responsibility |
| --- | --- |
| `core` | Shared types and `Config` (model / compaction / context window / small_model, all configurable) |
| `llm` | OpenAI-compatible streaming client + `ChatStream` trait + `MockChatClient` + token estimator |
| `store` | `Store` trait + RocksDB (`rdb`) implementation (embedded, atomic write batches, single-owner write lease), the sole outlet for all persistence |
| `session` | Runtime core: drain main loop, tool registration, subagent scheduling, plan bash guard, compaction, resume |
| `tui` | ratatui interactive UI (3-pane layout, subagent folding, steer/follow-up, plan/act switch) |
| `web` | axum HTTP + SSE session management (prompt admit / event stream / runtime switch / interrupt) |
| `control` / `server` | Fleet HTTP control plane and dedicated server binary |
| `node` / `worker` / `agent` | Node connection, execution adapters, and dedicated node binary |
| `local` / `ctl` | Local interactive and headless frontend, Fleet management CLI |
| `brain` / `dag` / `team` / `todos` | Orchestration capabilities and execution contracts |

**Key abstractions:**

- **`Store` trait** (`crates/store/src/store.rs`) — unified CRUD for sessions / messages / inputs / events / subagent_tasks; the only seam for swapping storage backends.
- **`ChatStream` trait** (`crates/llm/src/stream.rs`) — implemented by both `ChatClient` (real) and `MockChatClient`, enabling zero-token deterministic testing of the session runtime.
- **drain semantics** (`crates/session/src/runner.rs::run_loop`) — promotes steer at every turn boundary; when idle it consumes exactly one queued item; a doom-loop guard (`DOOM_THRESHOLD=20`) breaks consecutive empty turns.

## 📖 Command Reference

```
opencoder [OPTIONS] [PROMPT]...        # Default: enter the TUI
opencoder run <PROMPT>                  # Headless one-shot run
opencoder tui                           # Explicitly launch the TUI
opencoder-server --host <ADDR> --port <PORT> --token-file <PATH>  # Fleet server
opencoder-agent --remote <URL> --name <NAME> --token-file <PATH> # Fleet execution node
opencoder-cli --help                    # Fleet management CLI
opencoder config [show]                 # Inspect merged config
opencoder models                        # List known models
opencoder session <list|show|delete>    # Session management (show --json is a deep-inspection view)

Global options:
      --workdir <PATH>         Working directory
      --prompt-file <PATH>     Custom role prompt (full file text); a bash+subagent preamble is appended
  -s, --session <ID>           Resume a specific session
      --continue               Resume the most recent session in the current workdir
      --fork                   Copy before resuming; the original session is left unchanged
  -v, --verbose                Verbose logging
```

## ⚡ Performance

| Metric | Measured | Target |
| --- | --- | --- |
| Cold start (`--help`, 5 samples) | **~9 ms** | < 50 ms |
| Binary size | **~42 MB** (41,662,904 B, thin-LTO + strip, RocksDB statically linked) | — |
| Append 1000 messages (WriteBatch) | 5.6 ms → **0.006 ms/msg** | < 2 ms/msg |
| Load 1000 messages | **1.7 ms** | < 50 ms |
| List 200 sessions | **4.1 ms** | < 100 ms |

Threshold-contract tests live in [`store_perf.rs`](crates/store/tests/store_perf.rs) (run with `--release` for the measured figures); cold start is a 5-sample `--help` timing.

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
| Session storage | SQLite (WAL) + Drizzle ORM | RocksDB (`rdb`) + hand-written `Store` trait | Different storage models; see [storage comparison](#-storage) below |

> **Cold-start token overhead** methodology: take each agent's first request body sent to the LLM, count tokens in the `system` message + the `tools` array (the fixed first-turn cost, incurred before any user input) with the `cl100k_base` BPE. Measured independently from the other `--help` runtime-baseline rows above. This is a fixed cost re-paid on every conversation turn.

> The opencoder column above is a Turso-era snapshot (2026-07-19). After the move to RocksDB (rdb), re-sampled on 2026-11-07: cold start ~9 ms, binary ~42 MB (RocksDB statically linked, still a single native Rust binary); storage throughput is in the Performance table above.

### 🗄️ Storage

The storage models differ: opencode embeds a single SQLite file (WAL mode), while OpenCoder's main store is a RocksDB (`rdb`) directory:

| Dimension | opencode `1.17.8` | opencoder `0.1.0` |
| --- | --- | --- |
| Database | SQLite (embedded in bun) | RocksDB (`rdb`, embedded local directory) |
| On-disk shape | `opencode.db` + `.db-wal` + `.db-shm` single-file family | `<stem>.rdb` directory (e.g. `opencode.db.rdb`) with LSM-level files |
| Write model | WAL transactions (`journal_mode=WAL` and other PRAGMAs) | One atomic `WriteBatch` commit per mutation; a failure leaves no half-written batch |
| Concurrency model | Many readers / one writer (WAL) | Single owner / write lease (shared handle in-process + cross-process `.owner.lock`) + snapshot reads |
| Access layer | Drizzle ORM (JS, runtime migrations via `migration` table) | KV data model + `Store` trait (`crates/store/src/store.rs`) |
| Migration | Drizzle migration (runtime diff against `migration` table + `meta/_journal.json`) | Versioned KV key families; legacy SQLite session stores go through an explicit migration command |
| DB file naming | `opencode.db` (varies by channel, e.g. `opencode-dev.db`) | `<data_dir>/<stem>.rdb` (one directory per workdir) |

> Ontology uses an independent SQLite backend and does not follow the main store's move to `rdb`.

> Note: end-to-end task performance is LLM-dominated (first-token latency is the provider RTT); under the same model both projects are essentially neck-and-neck in actual task throughput. The comparison above isolates the **runtime baseline** — the cost the runtime itself imposes regardless of the LLM.


---

## 🧪 Development & Testing

This project strictly follows the development rules under [`rules/`](rules/): every platform feature must have corresponding tests, and a full regression run with a changelog + test manifest is required before each iteration ends.

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
│   ├── store/     # Store trait + RocksDB (`rdb`) implementation
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

Thanks to the following open-source projects:

- [ripgrep](https://github.com/BurntSushi/ripgrep) — content search backend in notepad (falls back to `grep -rn` when unavailable)
- [tmux](https://github.com/tmux/tmux) — managed sessions for `opencoder ts` (sessions survive SSH disconnects and reattach on reconnect)

Both are independent external projects: opencoder only invokes them at runtime and does not bundle or redistribute them.

## 📄 License

[MIT](LICENSE)
