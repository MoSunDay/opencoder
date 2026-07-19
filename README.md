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
  "provider": "openai",
  "model": "glm-4.6",
  "small_model": "glm-4.5-air",
  "context_limit": 128000,
  "max_tokens": 8000,
  "reasoning_effort": "medium",
  "compaction": { "auto": true, "context_threshold": 0.8 }
}
```

### 三种使用方式

```bash
# 1) 交互式 TUI
opencoder

# 2) 无头一次性运行，输出到 stdout
opencoder run "用 Rust 实现一个 LRU cache 并写测试"

# 3) 启动服务端（集中存储 + LLM 网关 + SSE），另一台机器用 client 接入
opencoder server --host 0.0.0.0 --port 8080
opencoder client --remote http://127.0.0.1:8080 "总结这个仓库的架构"
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
opencoder server [--host] [--port]      # 服务端（别名：serve）
opencoder client --remote <URL> <PROMPT># 远程瘦客户端
opencoder config [show]                 # 查看合并后的配置
opencoder models                        # 列出已知模型
opencoder session <list|show|delete>    # 会话管理（show --json 为深度观测面）

全局选项：
  -m, --model <MODEL>          指定主模型
      --small-model <MODEL>    指定 small model（title 生成等）
      --agent <explore|build>  指定 agent 类型
      --workdir <PATH>         工作目录
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

## 📄 License

[MIT](Cargo.toml)
