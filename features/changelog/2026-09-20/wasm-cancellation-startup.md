Commit: 1adc6126c1e36ff927a28fcf2b17ace4891e37be

# Wasm 初始化期间的取消

Dynamic DAG 全工作区回归在较快的临时文件系统中暴露出取消竞态：Wasm 的 epoch 时钟在线程内先处理取消，随后 Store 才安装相对截止点，提前发生的取消跳变因而失效，忙循环可能继续运行到默认的 24 小时截止点。真实 DAG 取消用例在 180 秒后失败，见 `/tmp/dynamic-memory-before-epoch.log`。

现在先设置 Store 截止点，再启动 epoch 时钟。执行前已取消的令牌直接返回 `Cancelled`，不进入 guest；取消结果使用同一个纯函数构造。未放宽超时或取消断言。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 执行前已取消，不允许写 guest 产物 | `pre_cancelled_token_never_runs_guest` | [Wasm 测试](../../../crates/dag-runtime/src/exec/wasm/tests.rs) |
| 执行中取消映射为 Cancelled | `cancel_token_maps_to_cancelled_outcome` | [Wasm 测试](../../../crates/dag-runtime/src/exec/wasm/tests.rs) |
| 显式超时继续中断忙循环 | `timeout_secs_traps_via_epoch_deadline` | [Wasm 测试](../../../crates/dag-runtime/src/exec/wasm/tests.rs) |
| Server 取消真实忙循环，收齐步骤回执并阻断下游 | `cancel_mid_run_and_agent_step_failure` | [进程验收](../../../tests/dag_e2e/cancel_fail.rs) |

新增测试在旧实现上失败：已取消令牌仍执行 guest、写入 `output.json` 并返回 `Done`（`/tmp/dynamic-epoch-before.log`）。修复后 Wasm 相关 14 项通过（`/tmp/dynamic-epoch-after.log`）；真实取消用例连续三次通过，分别耗时 0.88、0.58、0.64 秒，动态 Agent/Wasm 的实际 runc 用例也通过（`/tmp/dynamic-final-process.log`）。全工作区结果记录于 [Dynamic DAG 验收](dag-dynamic-instances.md)。
