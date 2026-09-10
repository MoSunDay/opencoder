# e321f107 发布后跟进：worker fixture 中性化 + workspace 全量补验

日期：2026-09-10 · 基线：`e321f107`（已发布，origin/main）

## 变更

- `crates/worker/tests/runner_dispatch.rs`：fixture 步骤名 `workflow` → `diagnose`，
  两处 lockstep 更名（`/api/dag/defs` spec 步骤名 + `/api/executions/{id}/artifact?step=` 查询）。
  对齐 `e321f107` 在 `docs/registered-runners.md` 已确立的中性词汇；
  纯测试 fixture 更名，无产品代码 / wire 值变化。

## 发布后全局补验（评审遗留 TODO #1 的可执行部分）

主工作树被并发会话未提交半成品（core `mod dag;` 接线中）占用且红线，
沿用发布轮既定等价补偿：隔离工作树（detached @ e321f107 + 本改动），
独立 target 目录 `/tmp/postrelease-target`。

- `cargo test -p opencoder-worker --test runner_dispatch`：1/1 通过（15.83s）
- `cargo build --workspace --bins`：通过（fleet e2e 前置，daemon_smoke 依赖
  同目录 `opencode-server`/`opencoder-agent` 二进制，panic 信息自文档化）
- `cargo test --workspace --no-fail-fast`：368 个测试二进制，5027 passed，
  1 failed —— `todo_workflows::dispatch_pins_env_and_reaches_node`（control e2e）
  503 "no ready online node" vs 期望 202，发生于宿主 load 250+（并发会话同机构建），
  判定为节点就绪时序 flake：
  - 单测复跑：1/1 通过（3.83s）
  - 整个 `--test e2e` 二进制复跑：168/168 通过（138.16s）
  flake 已双重闭合，非 `e321f107` 缺陷。
- core `mod dag;` 接线仍未完成（归属并发会话），接线落地后其所属迭代仍需
  再补跑一次 `cargo test --workspace`。

## 前置提交轻量确认（评审遗留 TODO #3）

- `fd9bd470`（fix(tui) notepad 视口 size_override 注入）：3 文件
  （keys.rs +7 / mod.rs +3 / notepad_scroll.rs 测试调整），
  生产路径默认 `None` 回退真实终端尺寸，行为不变，低风险；
  已有 changelog `fa3dd805` 记录。✅
- `fa3dd805`（docs changelog）：纯文档 +24 行，无代码。✅
- 两者均已随 `e321f107` 推送入库（origin/main 祖先）。

## 测试清单

| 命令 | 结果 |
|---|---|
| `cargo test -p opencoder-worker --test runner_dispatch` | 1/1 ok |
| `cargo build --workspace --bins` | ok |
| `cargo test --workspace --no-fail-fast` | 368 bins / 5027 ok / 1 env-flake |
| `cargo test -p opencoder-control --test e2e todo_workflows::dispatch_pins_env_and_reaches_node`（复跑） | 1/1 ok |
| `cargo test -p opencoder-control --test e2e`（复跑） | 168/168 ok |
