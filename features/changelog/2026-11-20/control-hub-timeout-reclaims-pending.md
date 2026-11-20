# Control Hub：RPC 超时回收 pending 槽位与容量声明

一句话：Hub 的 `call_with_timeout` 超时分支不再泄漏 `pending` 条目与容量
声明（`release_claim`），慢节点无法耗尽全局 1024 派发上限导致全队
"control channel busy"。

## 背景与缺陷

`crates/control/src/transport/hub.rs` 的 `call_with_timeout` 在超时后只返回
504（"node request timed out; retry using the same execution id"），但既不
删除已插入的 `state.pending` 条目，也不释放 `release_claim` 的容量声明；
此前条目只能靠迟到回复（`resolve`）或节点断开（`detach`）回收。一个保持
连接却从不回复的节点会永久占用 pending 槽位与 reservation：全局 pending
上限为 1024，少量慢节点即可把上限吃满，随后所有调用在派发前就被 503
"control channel busy" 拒绝，全队派发停摆。

## 变更

- 超时分支（`_ =>` 臂）重新加锁 `state`，`pending.remove(&id)`；若被移除
  条目带 `execution`，调用既有的 `release_claim`（覆盖 reservation 与
  `reserved_loops`）后返回原 504 文案，客户端重试语义不变。
- 迟到回复命中 `resolve` 的 miss 路径（entry 已不在 → 返回 `None`）被
  丢弃；`resolve` 其余语义未改动。
- 代码附注释说明：慢节点不得劫持全局 pending 槽位与容量声明，迟到回复由
  `resolve` 丢弃。
- 测试改写：原 `late_failure_cannot_reject_an_inflight_create_or_leak_claims`
  中“超时 create 保留结算身份”的断言已不符合新契约，改写为
  `timed_out_call_releases_pending_and_claim_and_drops_late_reply`（保留
  第二个并发调用、冲突拒绝回复与 detach 兜底等有效部分）。

## 变更清单

- `crates/control/src/transport/hub.rs` — 超时分支回收 pending 条目与容量
  声明（600 → 613 行）。
- `crates/control/src/transport/hub_tests.rs` — 改写超时契约测试、新增
  连续超时不累积条目测试（365 → 427 行）。
- 本 changelog。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 超时回收 pending + 容量声明、迟到回复被丢弃 | `timed_out_call_releases_pending_and_claim_and_drops_late_reply` | `crates/control/src/transport/hub_tests.rs` |
| 连续无响应超时不累积 pending、同 id 重试仍 200 | `unanswered_timeouts_do_not_accumulate_pending_entries` | `crates/control/src/transport/hub_tests.rs` |

既有回归（同文件，全部保持通过）：`creation_rpc_uses_the_slow_admission_window`、
`only_private_file_probes_use_the_digest_window`、
`legacy_node_cannot_join_or_receive_harness_assignments`、
`initial_report_gates_calls_and_scheduling_until_complete`、
`old_generation_cannot_touch_or_complete_new_connection`、
`post_operation_snapshot_precedes_capacity_claim_release`、
`delta_membership_preserves_unmentioned_acceptances_and_reservations`、
`unregister_sweeps_per_node_acceptances_and_reservations`。

## 实测

```text
cargo clippy -p opencoder-control --all-targets -- -D warnings   # exit 0，零警告
cargo test -p opencoder-control hub                              # 11 passed / 0 failed
                                                                 # （10 个 transport::hub 单测 + 1 个文档测试）
```

- 触及文件行数：`hub.rs` = 613 行、`hub_tests.rs` = 427 行（上限 800）。
- **未运行全量测试**：未运行 `cargo test --workspace`、SPA 全量测试或
  全站 UI 验收；仅运行上述定向用例。

## 回滚

- 直接 revert 本次提交；无 schema、协议、配置或依赖迁移。
- 回滚后超时条目重新依赖迟到回复或断开回收，慢节点泄漏缺陷随之恢复。
- 回滚时删除本 changelog。
