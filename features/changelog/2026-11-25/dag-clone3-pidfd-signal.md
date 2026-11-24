# DAG clone3 收尾：按 pidfd 精确信号，刷新规则 04 与 dag-runtime 索引溯源戳

一句话：clone3 清理对记录的 init 改经 pidfd 发送信号（pidfd 钉住进程后再核对 starttime），
彻底消除 pid 复用窗口；规则 04 与 dag-runtime 索引的 `Commit:` 溯源戳刷新为 `dd29777be`。

## 背景

- `dd29777be` 评审的问五指出 `clone3::kill` 为「存活检查 + `kill(pid)`」，检查与信号之间
  仍有理论上的 pid 复用窗口。
- 用户决策：本次收尾免测提交并授权推送远端——不运行 dag-runtime 全 target、e2e 双矩阵、
  worker 矩阵与两节点验收；因此本记录不宣称该增量可上线，可上线基线仍是 `dd29777be`。
- 问五中的 `/etc/hostname` 双后端对齐需要 e2e 观测才能决定，免测下顺延。

## 变更

- `crates/dag-runtime/src/sandbox/clone3/mod.rs`：`kill()` 先 `pidfd_open` 钉住进程，再读
  starttime 核对身份，经 `pidfd_send_signal` 发送；pidfd 不可得（非 ESRCH，如描述符耗尽）
  时记录警告并回退到原「身份检查 + `kill`」路径；进程已退出（ESRCH）不是错误。
- `rules/04-dag-execution-contract.md`、`agents/workflow/dag-runtime/index.md`：`Commit:`
  溯源戳刷新为 `dd29777be`（规则文字与索引内容均在该提交最后一次变更）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 只信号记录的那一次进程化身；陈旧身份与已退出进程均无副作用 | `kill_reaches_the_recorded_incarnation_only` | `crates/dag-runtime/src/sandbox/clone3/mod.rs` |

## 验证

- `cargo clippy -p opencoder-dag-runtime --all-targets -- -D warnings` 零告警；
  `cargo test -p opencoder-dag-runtime --lib sandbox::clone3::tests` 通过（含新增单测）。
- 按用户决策未运行：dag-runtime 其余 target、e2e `dag_e2e` 双矩阵、worker `harness_matrix`、
  两节点验收；不宣称该增量可上线。
- 测试范围：仅本次改动的编译检查与 clone3 模块单测；未运行全量 workspace 测试。
