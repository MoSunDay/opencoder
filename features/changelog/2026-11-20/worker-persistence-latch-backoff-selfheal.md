# Worker：persistence 闩锁改为指数退避自愈

一句话：节点 `persistence_error` 闩锁从"只写不清、一次瞬态 DB/NFS 抖动即
永久停摆派发"改为有界指数退避探测——退避窗口内跳过派发，一次完整成功的
派发周期即清除节点闩锁与项目依赖闩锁，调度自动恢复。

## 背景与缺陷

`crates/worker` 的持久化闩锁（`Inner.persistence_error` 与项目依赖侧
`Deps.persistence_error`）在全仓库内只有赋 `Some` 的写入，没有任何
`= None` 的清除路径：

- `operations/queue/mod.rs`：`dispatch_owned` 错误臂、调度循环错误臂直接
  写 `Some(...)`；`dispatch_locked` 开头只要任一闩锁非空就直接
  `return Ok(0)`，队列里的 pending 工作从此永远不被派发。
- `operations/launch.rs`：终态落盘失败、`finish_slot` 失败同样写死闩锁。
- `workloads/project.rs`：每次轮询把项目依赖闩锁原样拷贝进节点闩锁。

后果：一次瞬态的数据库 / NFS 抖动（例如一次 `finalize` 超时）就会让节点
**永久且静默**地停止派发已排队的工作，直到进程重启；节点快照里虽有错误
上报，但没有任何自动恢复路径。休眠门禁（`can_hibernate`）同样被永久
卡住。

## 变更

新增 `crates/worker/src/runtime/persistence.rs`（纯函数模块，接收
`&Worker`）：

- `RETRY_INITIAL = 1s`、`RETRY_MAX = 60s`；`backoff(attempts)` 纯函数给出
  1s、2s、4s … 封顶 60s（移位指数封 6，防溢出）。
- `record(worker, message)`：失败时递增 `attempts`、按退避排定
  `next_probe_at`、写入节点闩锁并 `tracing::error!` 记录。
- `blocked(worker)`：任一闩锁有错且未到探测期限 → 派发循环本轮不碰存储；
  仅依赖侧闩锁且无排定探测（未经 `record`）视为立即可探测。
- `active(worker)`：任一闩锁有错即为真，不看期限（供休眠门禁 / 快照）。
- `recover(worker)`：清空节点闩锁与项目依赖闩锁、重置退避计划——补上
  缺失的 `= None` 路径。从派发侧清依赖闩锁的依据：一次完整成功的派发
  周期端到端走过了 journal / store / todo 恢复的持久化；真正仍然坏掉的
  存储会在下一次失败时重新闩锁。

调用点：

- `dispatch_locked` 早退条件改为 `persistence::blocked(worker)`；循环
  正常走到末尾（本轮全部条目干净落盘）即调用 `persistence::recover`。
  admission 错误 / stopping 早退不触发恢复，语义不变。
- `dispatch_owned` 与调度循环错误臂改走 `persistence::record`，错误文案
  保持 `pending dispatch: {error:#}`。
- `launch.rs` 两个错误臂改走 `record`，文案保持 `execution {id}: …` 与
  `capacity completion {id}: …`。
- `workloads/project.rs` 轮询到依赖侧错误时改调
  `record(worker, "project persistence: {error}")`，`bail!` 文案不变。
- `can_hibernate` 两处直读改为 `!persistence::active(self)`；依赖侧
  require 失败仍按"未证明干净"拒绝休眠（与原直读一致）。
- `Inner` 新增 `persistence_retry: Mutex<PersistenceRetry>`（struct 定义
  于 persistence.rs，构造时初始化在 `persistence_error` 旁）。
- 未改 `operations/create.rs`（闩锁期间继续拒绝新 Create，保守语义，
  派发周期独立运行、无鸡生蛋问题）与 `service.rs` 快照直读；未重构
  `crates/project`（其闩锁由本恢复路径清除）。

## 变更清单

- `crates/worker/src/runtime/persistence.rs` — 新增，96 行（退避常量、
  `backoff`/`record`/`blocked`/`active`/`recover`、`PersistenceRetry`）。
- `crates/worker/src/runtime/persistence_tests.rs` — 新增，126 行。
- `crates/worker/src/runtime/mod.rs` — 挂载 `pub(crate) mod persistence;`
  与测试模块（42 → 45 行）。
- `crates/worker/src/state.rs` — `Inner` 新增 `persistence_retry` 字段与
  初始化（562 → 564 行）。
- `crates/worker/src/operations/queue/mod.rs` — 早退条件收敛为
  `blocked`，成功周期末尾 `recover`，错误臂改 `record`
  （414 → 406 行）。
- `crates/worker/src/operations/launch.rs` — 两个错误臂改 `record`
  （169 → 173 行）。
- `crates/worker/src/workloads/project.rs` — 拷贝改 `record`（49 行）。
- `crates/worker/src/runtime/capacity.rs` — `can_hibernate` 直读改
  `active`（255 → 249 行）。
- 本 changelog。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 退避序列 1s→2s→…→32s，≥7 次封顶 60s，`u32::MAX` 无溢出 | `backoff_progresses_and_caps` | `crates/worker/src/runtime/persistence_tests.rs` |
| 注入一次 DB 错误后：窗口内派发被阻断且闩锁保持；期限过后一轮健康派发清除闩锁、`attempts` 归零 | `latched_worker_recovers_after_backoff_window` | `crates/worker/src/runtime/persistence_tests.rs` |
| 仅依赖侧闩锁立即可探测；一轮成功派发同时清除依赖侧闩锁 | `successful_dispatch_clears_the_project_dependencies_latch` | `crates/worker/src/runtime/persistence_tests.rs` |

## 实测

```text
cargo clippy -p opencoder-worker --all-targets -- -D warnings -A clippy::needless_range_loop
                                          # exit 0，worker crate 零警告
                                          #（-A 仅因 dag-runtime 依赖中与本改动
                                          #  无关的存量 needless_range_loop，
                                          #  stash 验证 HEAD 上即失败）
cargo test -p opencoder-worker --lib persistence   # 4 passed / 0 failed
cargo test -p opencoder-worker --lib queue         # 3 passed / 0 failed
cargo test -p opencoder-worker --lib capacity      # 8 passed / 0 failed
cargo test -p opencoder-worker --lib state::       # 12 passed / 0 failed
```

- 触及文件行数：新增 96 / 126 行（≤400）；迭代文件最大 `state.rs` 564 行
  （≤800）。
- **未运行全量测试**：未运行 `cargo test --workspace`、SPA 全量测试或
  全站 UI 验收；仅运行上述定向用例。

## 回滚

- 直接 revert 本次提交；无 schema、协议、配置或依赖迁移。
- 回滚后闩锁恢复只写不清，瞬态存储抖动重新永久停摆节点派发。
- 回滚时删除本 changelog。
