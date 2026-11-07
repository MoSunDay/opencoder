# 存储口径单一事实源：事件框架回归纯内存、历史记录合并清理

一句话：事件框架精简为纯内存实现（移除未接生产的外部持久化后端及其
workspace 依赖），文档与仓库记忆统一为单一存储口径（Turso/SQLite WAL），
并合并清理了过时历史记录，保证后续任务检索上下文只有一个存储事实源。

## 变更清单

- `repo-memory.md` — event 模块描述改为「纯内存、零存储引擎依赖」，
  移除可选持久队列账本措辞；Commit 指针刷新至当前 HEAD。
- `agents/foundation/event/index.md` — 头部改为「默认纯内存、零存储引擎
  依赖；不附带任何持久化后端」；删除持久队列账本源码索引项与对应集成
  测试条目；内联测试清单收窄为 `src/domain/*`、`src/ledger.rs`；Commit
  指针同步刷新。
- 删除 `features/changelog/2026-11-06/` 下 event 可选持久队列账本条目 —
  主题即被移除的外部持久化后端，整体失效。
- 删除 `features/changelog/2026-11-07/` 下会话存储后端回退记录 —
  主题即被移除后端的引入与回退过程，整体失效。
- 删除 `features/changelog/2026-11-07/` 下回退残留清理记录 — 主题即该
  回退的依赖与文档收口过程，整体失效。
- 删除 `features/changelog/2026-11-07/` 下 README 存储措辞终清记录 —
  主题即该回退的措辞收口过程，整体失效。
- `features/changelog/2026-08-30/skill-injection-fallback-contract-reclosure.md`
  — 移除验证边界中对无关外部项目测试风暴的点名。
- `features/changelog/2026-09-15/tui-markdown-table-render.md` — 三处
  外部工作区绝对路径引用改为中性表述（「本地另一工作区」），会话/事件
  计数与 PID 证据保留。
- `features/changelog/2026-11-06/tui-compaction-summary-order.md` — 旁路
  修复条目与 clippy 注记改为「他人未提交的存储原型」中性表述。
- `features/changelog/2026-11-06/tui-live-step-focus.md` — 既有问题注记
  改为「存储原型改变了存储路径布局」中性表述。
- `features/changelog/2026-11-06/tui-live-turn-focus.md` — 既有问题注记
  改为「他人未提交的存储后端原型替换」中性表述，保留干净 worktree 证据。

## 边界说明

- 代码侧（`crates/`、`Cargo.toml`、`Cargo.lock`）的对应精简由同批代码
  任务落地，本条目只覆盖文档、仓库记忆与历史记录口径。
- 现行存储口径唯一：基础存储为 Turso（SQLite WAL），事件框架为纯内存，
  workspace 无第二存储引擎依赖。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 总线扇出与 `MemoryLedger` 语义 | `cargo test -p opencoder-event --test bus_ledger` | `crates/event/tests/bus_ledger.rs` |
| 游标重放、失败保留、分批、暂停恢复 | `cargo test -p opencoder-event --test consumer_pump` | `crates/event/tests/consumer_pump.rs` |
| 触发规划与派发逐监听器结果 | `cargo test -p opencoder-event --test executor` | `crates/event/tests/executor.rs` |
| 监听器注册校验与匹配顺序 | `cargo test -p opencoder-event --test registry` | `crates/event/tests/registry.rs` |
| 文档与仓库记忆无第二存储口径、无残留措辞、无死链 | 人工核验（全仓 grep 零命中 + 链接抽查） | `repo-memory.md`、`agents/`、`features/` |

- 定向执行：`cargo test --locked -p opencoder-event`（bus_ledger 6
  passed / 0 failed、consumer_pump 7 passed / 0 failed、executor 3
  passed / 0 failed、registry 9 passed / 0 failed）。
- 测试范围：定向（本次改动及其直接影响链路：event crate 直接链路 + 文档
  人工核验）；未运行全量测试。
