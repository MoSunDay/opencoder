# fa70eb7e 跟进：todo_workflows dispatch 503 flake 加固 + fleet smoke 前置固化

日期：2026-09-11 · 基线：`fa70eb7e`（origin/main）。承接 `postrelease-e321f107-followup.md`
遗留 TODO #2（503 flake 容忍）与 TODO #3（sibling-bin 前置固化）；纯测试基建，零产品代码变更。

## 变更

### TODO #2：dispatch 瞬态 503 测试容忍（`crates/control/tests/e2e/`）

- `support/http.rs`：新增 opt-in 助手 `Harness::dispatch`——仅对
  `503 "no ready online node can accept this execution"`（placement 选择失败，
  `executions/mod.rs` 选择器路径，发生于任何落库/Create 之前，重试无副作用）重试至
  60s 截止（覆盖 20s staleness 窗 + 数个 5s 心跳拍），其余回复原样透传。
  判定抽为纯函数 `transient_no_ready` + 常量 `TRANSIENT_NO_READY`，附单测
  （同状态不同错误 / 202 / 空 error 形状均不重试——不掩盖真实负路径）。
- `todo_workflows.rs`：两个正向 dispatch 站点换用 `h.dispatch`
  （`todos-run-1`、`todos-envrun-1`，即上轮 flake 位点）；400 负路径与仓库内
  其余 503 断言测试（`executions_submit` / `users_api` / `dag_dispatch_extra` /
  `compat_nodes` 等）保持 `h.req` 精确断言，不受影响。
- 新增确定性复现测试 `dispatch_retries_transient_no_ready_node`：
  `set_snapshot_opts(None, Some(false))` 强制 not-ready → 轮询 hub views 确认传播
  （≤心跳拍）→ 后台 300ms 翻回 ready → dispatch 首试必中 503、重试吸收瞬态窗口后
  202 + node journal 断言。上轮「16 倍超卖下心跳错过 staleness 窗」的环境 flake
  从此有受控回归位。

### TODO #3：fleet smoke sibling-bin 前置固化（根包 `tests/`）

- `tests/support/mod.rs`：补救文案抽为 `pub const FLEET_BINS_HINT`（panic 单源引用），
  模块文档新增 `# Prerequisite` 小节；`tests/daemon_smoke.rs` 头注补前置一行。
- 机制澄清（本轮实证）：`cargo test --workspace` 只把「自身包的 bins」uplift 到
  `target/debug/`，server/agent 包 bins 不 uplift——`cargo build --workspace --bins`
  先行是 load-bearing 步骤（既有 runbook 语义，非可选）。

### TODO #1（外部项核查）

core `mod dag;` 接线本轮核查仍未落地（`crates/core/src/lib.rs` 无 `mod dag;`），
其 workspace 补验继续归属接线迭代。

## workspace 全量 gate 与环境事件（如实披露）

隔离工作树 @ `fa70eb7e` + 本改动，独立 target `/tmp/oc-fa70eb7e-target`。

- `cargo test --workspace --no-fail-fast`：368 测试二进制，345 个首轮全绿
  （含 `opencode-control --test e2e` 170/170 = 原有 168 + 新增 2）。
- 23 个二进制首轮失败，逐项归因后全部闭合，**无一为代码缺陷**：
  1. **sibling 前置（3 bins）**：本轮 gate 未先跑 `--bins` 构建 → daemon_smoke /
     nodes_smoke_proc / running_mode_switch_e2e 于 `sibling_bin` 自文档化 panic。
     补 `cargo build --workspace --bins`（3m43s）后 1/1、1/1、2/2 全绿。
  2. **磁盘守卫级联（19 bins，worker/project 系）**：隔离 target ~34G + 并发会话
     同机构建把可用空间压至 17.6%（< worker admission 的 20% 阈值），节点
     `node storage low` fail-closed 拒绝（按设计工作）。清理主树 target 中
     >30min 陈旧 incremental 会话缓存（+23G，纯缓存、活跃写入会话未动）后
     复跑全绿：worker --lib 30/30、platform 12/12、durable_execution 6/6、
     durable_lifecycle 5/5、harness_settings_queue 4/4、workloads 5/5、
     runner_dispatch 1/1、artifact_stream/control_pagination/fleet_index_contract/
     fleet_channel/harness_codex/harness_matrix/initial_input_recovery/
     internal_session_index/project_replay/query_pagination/queue_project 各 1..3/1..3、
     executor_team_dag_brain 8/8（忙时复跑 2 失败为负载时序，静置 1.96s 全绿）。
  3. **responses_cli（1 bin）**：CLI 收尾阶段（30s 截止的 title 生成 + 落盘 flush）
     在并发构建 ~8x CPU 超卖下整体拉伸至 ~80s，撞测试 30s/进程上限。手工复现诊断：
     两轮 LLM 往返 1.3s 完成、e2e 行为正确（exit 0、edit 生效、输出符合）；负载
     回落后复跑 1/1 ok（9.09s）。环境时序，非缺陷。
- 过程披露：实施中两条 python 补丁因未显式指定 workdir 误写主树
  `tests/support/mod.rs`、`tests/daemon_smoke.rs`（当时 porcelain-clean），当轮
  `git restore` 还原至 HEAD 字节等价，主树零残留；未触碰并发会话任何脏文件。

## 测试清单

| 命令 | 结果 |
|---|---|
| `cargo test -p opencoder-control --test e2e todo_workflows`（定向） | 8/8 ok（含新增复现测试，105.62s 重载） |
| `cargo test --workspace --no-fail-fast`（隔离树全量） | 368 bins，345 首轮绿，23 环境性失败全闭合 |
| `cargo test -p opencoder-control --test e2e`（gate 内含） | 170/170 ok（support::http predicate 单测 + 复现测试在内） |
| `cargo build --workspace --bins` | ok（3m43s，fleet smoke 前置） |
| `cargo test -p opencoder --test daemon_smoke / nodes_smoke_proc / running_mode_switch_e2e` | 1/1、1/1、2/2 ok |
| `cargo test -p opencoder --test responses_cli`（静置复跑） | 1/1 ok（9.09s） |
| worker/project 19 bins 逐个复跑 | 全绿（上文明细） |

相关语义：[Control](../../../agents/control/index.md)。
