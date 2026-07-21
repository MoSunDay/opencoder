<p align="center">
  <img src="logo/logo.png" alt="OpenCoder" width="220" />
</p>

<h1 align="center">OpenCoder</h1>

<p align="center">
  从零实现的 Rust 原生编码代理 · 单二进制 · 可替换的存储与 LLM 后端<br/>
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
  <strong>简体中文</strong> · <a href="./README.en.md">English</a>
</p>

---

> ## ⚠️ 风险提示：本项目 100% 信任模型，不做任何权限拦截
>
> **OpenCoder 没有任何权限拦截或沙箱机制。** 一旦启动，模型将被授予与当前操作系统用户**完全等价**的权限：它可以直接读写、修改、删除你账户下可达的任意文件，也可以执行任意 shell 命令（包括 `rm`、`git push --force`、写数据库、联网下载等），**且不会弹出二次确认**。
>
> 换句话说，**你运行 OpenCoder，等于把这台机器的全部权限交给了模型。**
>
> 在使用前，请务必：
>
> - 📌 **先衡量风险**：仅在你愿意承担「模型可能误删/改写任意文件、执行任意命令」的前提下使用；
> - 💾 **做好重要数据备份**：对你关心的代码、数据、配置提前备份或纳入版本控制，并确认备份不在此进程可写的目录内；
> - 🔐 **隔离运行环境**：建议在容器、虚拟机或专用低权限账户中运行，切勿直接在生产服务器、含敏感凭证（API Key / 私钥 / 数据库连接串）的工作目录上运行；
> - 👀 **全程留意**：headless `run` 与 `serve` 模式下命令会自动执行，请确保工作目录与权限范围在你可控之内。
>
> **如果你不信任模型，请不要在没有充分备份和隔离的情况下使用本项目。** 作者不对因模型行为造成的任何数据丢失或损害承担责任。

---

OpenCoder 是一个完全独立、从零实现的 Rust 原生编码代理。它以单一二进制 `opencoder` 提供 **交互式 TUI**、**无头一次性运行**、**集中式 HTTP/SSE 服务端** 与 **远程瘦客户端** 四种工作形态。所有上层逻辑只依赖两个抽象口子 —— `Arc<dyn Store>` 与 `Arc<dyn ChatStream>` —— 因此持久化层（libsql）与 LLM 后端（OpenAI 兼容）均可替换。

## ✨ 特性

- **🧠 多形态运行时** — TUI 交互、headless `run`、`server`（HTTP/JSON + SSE）、`client` 远程瘦前端，四种入口共享同一套 session 运行时。
- **🔄 会话恢复与分叉** — `--session <id>` / `--continue` / `--fork` 跨进程从 libsql 重建历史；title 由 small model 异步生成。
- **📦 Session 二进制导出/导入** — `session export/import` 以 `.opencoder` 二进制（`OPENCODR` magic）携带完整 subagent 树迁移会话，幂等且不导出 Config（API key 安全）。
- **🛠️ Subagent 调度** — `explore`（只读探查）与 `build`（实现执行）两类子代理，DB 追踪生命周期、可折叠查看。
- **📋 Plan / Act 双模式** — Plan 模式只读（bash 写操作被 `bash_guard` 拦截），切换到 Act 时清空上下文只保留最终计划。
- **🗜️ 自动压缩** — token 估算驱动的上下文压缩，`compaction.{auto,context_threshold,reserved,tail_turns,buffer}` 全可配置。
- **🎮 steer / followup 两段式投递** — 运行中可在 turn 边界即时插入 steer，idle 时消费恰好一条 queue。
- **🌐 无损事件回放** — SSE 事件 schema v2 迁移，`SessionEvent` 作为单一真相源，replay 完整重建工具块。
- **⚡ 高性能** — 冷启动 ~6 ms，二进制 9.3 MB（thin-LTO + strip）；libsql WAL 并发读写，千条消息追加 30 ms。

## 🚀 快速开始

### 安装

从源码构建（需要 Rust 工具链）：

```bash
git clone https://github.com/MoSunDay/opencoder.git
cd opencoder
cargo build --release
# 二进制位于 target/release/opencoder
```

或使用安装脚本：

```bash
curl -fsSL https://raw.githubusercontent.com/MoSunDay/opencoder/main/scripts/install.sh | bash
```

### 配置

在项目根目录或 `~/.opencode/` 放置 `opencoder.json`（环境变量与 CLI flag 优先级更高）：

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

`model` 格式为 `"{provider}/{model_id}"`，provider 名匹配 `providers` map 的 key（未匹配则回退到默认 `provider`）。`api_key` / header `value` 支持 `{ENV_VAR}` 环境变量间接引用。

### 三种使用方式

```bash
# 1) 交互式 TUI
opencoder

# 2) 无头一次性运行，输出到 stdout
opencoder run "用 Rust 实现一个 LRU cache 并写测试"

# 3) 启动服务端（集中存储 + LLM 网关 + SSE），另一台机器用 client 接入
opencoder server --host 0.0.0.0 --port 8080
opencoder client --remote http://127.0.0.1:8080 "总结这个仓库的架构"

# 4) 在 tmux 里跑 TUI：SSH 断线后会话存活，重连后 opencode ts 自动 reattach
opencode ts            # 新建/恢复 tmux 会话
opencode ts -l         # 列出受管会话（tmux 名 + 任务信息）
opencode ts -r <id>    # 恢复指定会话
```

## 🧱 架构

OpenCoder 是一个 Cargo workspace，由 8 个 crate 组成，依赖严格分层：

| Crate | 职责 |
| --- | --- |
| `core` | 共享类型与 `Config`（模型 / 压缩 / 上下文窗口 / small_model 全配置化） |
| `llm` | OpenAI 兼容流式客户端 + `ChatStream` trait + `MockChatClient` + token 估算器 |
| `store` | `Store` trait + libsql 实现（WAL，本地嵌入），所有持久化的唯一出口 |
| `session` | 运行时核心：drain 主循环、工具注册、subagent 调度、plan bash 守卫、压缩、resume |
| `tui` | ratatui 交互界面（3 区域布局、subagent 折叠、steer/followup、plan/act 切换） |
| `web` | axum HTTP + SSE 会话管理（prompt admit / 事件流 / 运行时切换 / interrupt） |
| `client` | 远程瘦客户端：提交 prompt 并流式回放，本地不存储、不调 LLM |
| `cli` | clap 前端 + headless 运行时（run / tui / server / client / config / models / session） |

**关键抽象：**

- **`Store` trait**（`crates/store/src/store.rs`）— sessions / messages / inputs / events / subagent_tasks 的统一 CRUD，切换 SQLite 实现的唯一接缝。
- **`ChatStream` trait**（`crates/llm/src/stream.rs`）— `ChatClient` 与 `MockChatClient` 共同实现，使 session 运行时可零 token 确定性测试。
- **drain 语义**（`crates/session/src/runner.rs::run_loop`）— 每个 turn 边界提升 steer；idle 时消费恰好一条 queue；doom-loop 守卫（`DOOM_THRESHOLD=3`）打破连续空 turn。

## 📖 命令参考

```
opencoder [OPTIONS] [PROMPT]...        # 默认进入 TUI
opencoder run <PROMPT>                  # 无头一次性运行
opencoder tui                           # 显式启动 TUI
opencoder ts                            # 在 tmux 里跑 TUI（SSH 断线存活；-l 列出，-r <id> 恢复）
opencoder server [--host] [--port]      # 服务端（别名：serve）
opencoder client --remote <URL> <PROMPT># 远程瘦客户端
opencoder config [show]                 # 查看合并后的配置
opencoder models                        # 列出已知模型
opencoder session <list|show|delete>    # 会话管理（show --json 为深度观测面）

全局选项：
      --workdir <PATH>         工作目录
      --prompt-file <PATH>     自定义角色提示词（文件全文），末尾自动追加 bash+subagent 前言
  -s, --session <ID>           恢复指定会话
      --continue               恢复当前 workdir 最近会话
      --fork                   恢复前复制，原会话保持不变
  -v, --verbose                详细日志
```

## ⚡ 性能

| 指标 | 实测 | 目标 |
| --- | --- | --- |
| 冷启动（`--help`） | **~6 ms** | < 50 ms |
| 二进制大小 | **9.3 MB** | — |
| 追加 1000 条消息（事务） | 30.5 ms → **0.031 ms/条** | < 2 ms/条 |
| 加载 1000 条消息 | **2.4 ms** | < 50 ms |
| 列出 200 个 session | **0.95 ms** | < 100 ms |
| 全量确定性测试套件 | **~3 s / 384 测试** | — |

详见 [`docs/perf.md`](docs/perf.md)。

## 📊 与 opencode 对比

OpenCoder 与 [sst/opencode](https://github.com/sst/opencode)（TypeScript / Node SEA 实现）面向同一类「编码代理」需求，但运行时基线开销差异显著。下表为**实测运行时基线对比**（非端到端任务跑分），同机同负载可复现。

**测试环境：** Intel Xeon E5-2673 v3 @ 2.40GHz · 24 核 · Ubuntu 22.04 (Linux 6.8) · 2026-07-19

**测量方法：** 负载 = `--help`（最小工作负载，反映运行时本身开销、排除 LLM RTT）；峰值内存用 `/usr/bin/time -v` 的 *Maximum resident set size*；各项取样 5 次取中位数。

| 指标 | opencode `1.17.8` | opencoder `0.1.0` | 差异 |
| --- | --- | --- | --- |
| 运行时 | Node SEA（V8，单可执行应用） | Rust 原生单二进制 | — |
| 二进制大小 | **159 MiB**（166,885,504 B） | **11.1 MiB**（11,644,304 B） | opencode 大 **14.3×** |
| 冷启动峰值 RSS | **~197 MiB**（195–199 MB） | **~5.4 MiB**（5.2–5.6 MB） | opencode 高 **35×** |
| 冷启动耗时 | **~0.78 s** | **~6 ms** | opencode 慢 **~125×** |
| 冷启 token 开销 | **~13 k tok** | **~0.4 k tok** | opencode 高 **~32×** |
| 协议 | OpenAI 兼容 + ACP + MCP | OpenAI 兼容 + 子代理（explore/build） | — |
| 会话存储 | SQLite（WAL）+ Drizzle ORM | libsql（SQLite 兼容，WAL）+ 自写 `Store` trait | 均为 SQLite；见下方[存储对比](#-存储) |

> **冷启 token 开销**口径：取各 agent 首次发给 LLM 的请求体，计数其中的 `system` 消息 + `tools` 数组（首轮固定开销，发生在任何用户输入之前），用 `cl100k_base` BPE 计 token；与上表其余 `--help` 运行时基线指标独立测量。这一项是每一轮对话都重复支付的固定成本。

### 🗄️ 存储

两者**底层都是 SQLite**，落盘均为标准 `.db` + `.db-wal` + `.db-shm`（WAL 模式），差异在驱动层与表模型：

| 维度 | opencode `1.17.8` | opencoder `0.1.0` |
| --- | --- | --- |
| 数据库 | SQLite（bun 内嵌） | libsql `0.9.30`（SQLite 兼容，`core` + `libsql-sys`） |
| 访问层 | Drizzle ORM（JS，运行时迁移 `migration` 表） | 手写 SQL + `Store` trait（`crates/store/src/store.rs`） |
| 驱动语言 | JS/V8 ↔ SQLite C ABI | Rust ↔ libsql C ABI（零拷贝绑定） |
| WAL PRAGMA | `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`, `cache_size=-64000`, `foreign_keys=ON` | `journal_mode=WAL`（同样的并发模型） |
| 表（核心） | `session` / `message` / `step` / `agent` / `tool_use` / `snapshot` / `checkpoint` / `event` / `subagent` | `sessions` / `messages` / `session_inputs` / `session_events` / `subagent_tasks` / `schema_version` |
| 迁移 | Drizzle migration（运行时比对 `migration` 表 + `meta/_journal.json`） | `schema_version` 单值 + `CREATE TABLE IF NOT EXISTS` 幂等建表 |
| 库文件命名 | `opencode.db`（按 channel 可变，如 `opencode-dev.db`） | 由 workdir 决定（每 workdir 一个库） |

> 两者的 WAL 并发模型等价（多读一写、读不阻塞写），存储吞吐差异主要来自驱动层 —— opencode 经 Drizzle ORM + V8 跨语言，opencoder 直连 libsql C 绑定且无 ORM 转换层。

> 说明：端到端任务表现受 LLM 主导（首轮 token 延迟为 provider RTT），同一模型下两者任务能力等价；**差异集中在运行时基线** —— 内存占用、启动延迟、分发体积。对受约束环境（CI runner、容器、弱机）与多实例部署（按进程计的 RSS 直接决定并发密度），opencoder 的低开销优势可被直接摊销。

### 同一需求：贪吃蛇

以「用 Rust + crossterm 实现终端贪吃蛇」为对照任务，两者入口一致：

```bash
# opencode（Node SEA）
opencode run --model zhipuai-coding-plan/glm-5.2 "实现终端贪吃蛇..."

# opencoder（Rust 原生）
opencoder run "实现终端贪吃蛇..."
```

**实测对比（2026-07-19，同机同模型 glm-5.2，隔离工作目录，0.5 s 全程采样）：**
**口径：仅统计「非编译时段」—— agent 主进程本身，且无 cargo/rustc 子进程在跑（`npids<=1` 的样本）。两边完全对称，彻底排除编译贡献。**

| 指标 | opencode `1.17.8` | opencoder `0.1.0` | 差异 |
| --- | --- | --- | --- |
| 任务完成耗时（wall） | 172.3 s | 112.2 s | LLM 采样噪声大，仅参考 |
| 非编译样本占比 | 313 / 321（97.5 %） | 199 / 209（95.2 %） | 编译占比都很小 |
| **CPU 平均利用率** | **55.3 %**（持续高占用） | **0.13 %**（事件驱动，等 LLM 时空闲） | opencode 高 **~425×** |
| CPU 中位数（p50） | **44.6 %**（一半时间 ≥44 %） | **0.0 %**（一半时间完全 0） | 本质差异 |
| CPU p95 | 115.2 % | 1.9 % | opencode 高 **~61×** |
| CPU 峰值 | 2631.6 %（V8 GC/JIT，瞬时 ≈26 核） | 3.7 % | — |
| **Agent 进程 RSS 均值** | **451.7 MB** | **11.8 MB** | opencode 高 **~38×** |
| Agent 进程 RSS 峰值 | 557.5 MB | 12.1 MB | opencode 高 **~47×** |
| 结果 | 编译通过，242 行 | 编译通过，351 行 | — |

> **关键：opencode 的高 CPU 与编译无关（已用数据自证）。** opencode 的 CPU 峰值 2631.6 % 那个样本经核实 `npids=1`（此刻并无 cargo 子进程在跑），是纯 V8 GC/JIT 爆发；且把编译时段全部剔除后，opencode 的 CPU 均值不降反略升（全程 54.4 % → 非编译 55.3 %）。换言之 opencode 在「等 LLM 回包」「编排工具」这些与编译无关的时段里，自身 V8 运行时仍维持 ~55 % 的持续高 CPU。opencoder 则相反 —— 非编译时段 CPU 均值 0.13 %、中位 0，agent 在等待时基本完全静默。

> **口径与可信度说明：**
> - 表中所有数字均来自**非编译时段**样本（`npids<=1`，agent 主进程且无 cargo/rustc 子进程），对两边完全对称。编译时段（opencode 占 2.5 %、opencoder 占 4.8 %）已全部剔除。
> - **wall-time 受 LLM 采样随机性影响**：本次 opencode 172.3 s、opencoder 112.2 s，前一轮是 125.7 s / 79.0 s，方向相反，故仅作参考，**不宜当稳态结论**；CPU 利用率与 RSS 才是运行时基线的稳定差异。
> - opencode `run` 自带 V8 运行时，维持常驻高 CPU/RSS；opencoder 编译为原生异步运行时，等待回包时 CPU 降到 ~0。
> - opencode 另有常驻 server daemon（≈285 MB）未计入；如长期 server 模式，其常驻开销更高。
> - 原始 CSV 证据（含 `root_*` / `tree_*` / `npids` 六列）见 [`docs/bench/opencode-vs-opencoder-2026-07-19/`](docs/bench/opencode-vs-opencoder-2026-07-19/)。

---

## 🧪 开发与测试

本项目强制遵循 [`rules/`](rules/) 下的开发规则：每个业务功能必须有对应测试，每轮迭代结束前跑全量回归并附 changelog + 测试清单。

```bash
# 单元 + 集成测试（确定性，零网络）
cargo test --workspace

# 真实模型端到端契约测试（~3–5 min，需 API key）
scripts/e2e-glm.sh
```

测试分层规范见 [`rules/03-test-pyramid.md`](rules/03-test-pyramid.md)。

## 📁 项目结构

```
opencoder/
├── crates/
│   ├── core/      # 共享类型与 Config
│   ├── llm/       # LLM 客户端 + ChatStream trait
│   ├── store/     # Store trait + libsql 实现
│   ├── session/   # 会话运行时核心
│   ├── tui/       # ratatui 交互界面
│   ├── web/       # axum HTTP + SSE
│   ├── client/    # 远程瘦客户端
│   └── cli/       # clap 前端 + headless 运行时
├── docs/          # 性能 profile 等文档
├── features/      # 能力地图 + 按日期归档的 changelog
├── rules/         # 开发规则（测试 / 回归 / 分层）
├── scripts/       # 安装脚本、e2e 测试
├── logo/          # 项目 Logo
└── src/main.rs    # 二进制入口
```

## 🙏 致谢

本项目的可选能力站在以下优秀项目的肩膀上：

| 项目 | 贡献方式 |
| --- | --- |
| [obscura](https://github.com/h4ckf0r0day/obscura) | 底层浏览器引擎依赖（feature-gated `browser`）：基于 deno_core / V8 的无头渲染，驱动 `web_fetch` / `web_search` 的 JS 执行与反爬。 |
| [agent-browser](https://github.com/h4ckf0r0day/agent-browser) | `crates/session/src/tools/web_read.rs` 的内容提取算法参考：markdown Accept 协商、`.md` 回退、`llms.txt` / `llms-full.txt` 祖先爬取、可读正文抽取。 |
| [cua](https://github.com/h4ckf0r0day/cua) | `crates/core/src/computer_use.rs` 的 computer-use 循环参考：perceive → act 闭环被提炼为原生的 `ComputerUseExecutor` trait + `ComputerUseLoop`。 |

以上三者均为独立实现：obscura 作为 rev-pinned git 依赖接入；agent-browser 与 cua 的算法/思路被移植 / 蒸馏为纯 Rust（未引入其源码依赖）。

## 📄 License

[MIT](LICENSE)
