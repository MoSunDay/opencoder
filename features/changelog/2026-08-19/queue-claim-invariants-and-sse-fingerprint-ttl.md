Commit: (working-tree, post-75d6866；子项 1/3 的已跟踪文件改动被并行进程的 75d6866 一并收入，子项 2 与新增文件仍在工作树未提交)

# 批次二 P2-4：queue claim 不变量守卫 + SSE 指纹 TTL + drain claim 持续失败可见化

## 背景

三个独立但同类的静默缺陷，均围绕「已经发生的事被当作没发生 / 没发生的事被当作发生了」：

1. **store 层**：`session_inputs.recorded` 生命周期里，`claim_next_queue` / `promote_next_queued`（queue drain 的消费路径）只检查 `promoted_seq IS NULL`，不检查 `recorded`。而错误恢复路径 `unpromote_inputs` 只清 `promoted_seq`、不动 `recorded`——一条「已消费进 transcript（recorded=1）又被恢复为 pending」的行会被 drain 再次 claim，prompt **重复入 transcript**。
2. **web SSE 层**：tier-(2) 内容指纹去重集合 `seen`（P0-1 修复后从真正的 overlap 窗口播种）在流生命周期内永不清理。任何后续 live 事件若 (kind,data) 恰与 overlap 窗口指纹相同，会在**整条流**的生命周期内被永久误删。
3. **session 层**：drain 的 `claim_next_queue` 持续失败（初次 + 单次重试都 Err）只 `warn!` 到日志，按 Empty 处理 → run_loop 报 Done，pending 行搁浅且事件流上**没有任何错误提示**（UI 完全静默）。

## 根因

- `crates/store/src/libsql_store/inputs.rs`：bulk `promote`（Steer 批量路径）在再提升时**故意**重置 `recorded=0`（消费动作在 promote 之后统一发生，见既有测试 `promote_resets_recorded_marker_on_repromotion`）；但 queue drain 的单条 claim/promote 是「claim 即将消费」语义，行到达该路径时 `recorded=1` 只可能意味着**已经消费过**——两函数的 SELECT/UPDATE 没有区分这一点。
- `crates/web/src/api.rs::get_events`：指纹在首次匹配时 `remove`（自清理），但从未匹配上的指纹（overlap 窗口事件没有对应的 live 重复广播）永远留在集合里。缺少「overlap 窗口何时确定性结束」的判定。
- `crates/session/src/runner/drain.rs::claim_one_queued`：重试一次仍 Err → `warn!` + `None`。日志对 UI 不可见，违反「搁浅必须可观测」。

## 变更

- `crates/store/src/libsql_store/inputs.rs`
  - `promote_next_queued` / `claim_next_queue` 的 **SELECT 和 UPDATE 均加 `AND recorded = 0`**。SELECT 必须加：否则会选中 recorded 行、被守卫的 UPDATE 匹配 0 行、函数却仍谎报该行已提升（且每次调用重复选中同一行）。UPDATE 同时保留为纵深防御（风格与行 77 附近的 bulk `promote` 及 `recover_orphans` 的守卫一致）。函数 doc 注释补充不变量说明。
  - 范围刻意限定在这两个 queue-drain 函数：bulk `promote`（Steer 路径，生产唯一显式调用是 `Delivery::Steer`）保持「再提升即重置 recorded=0」的既有语义不变；`pending_inputs` 不加过滤（pending 投影仍如实反映行的物理状态，供 UI 镜像/审计）。
- `crates/web/src/api.rs::get_events` + 新模块 `crates/web/src/sse_dedup.rs`（拆分自 api.rs：原文件 799 行已顶到 800 帽，TTL 修复后超限，按职责把去重决策整体迁出）
  - live 流中**第一个通过去重检查（被转发）的 `done` 事件**处清空 `seen`：`done` 是一个 run 的最后一个事件，首个转发的 live `done` = overlap 窗口确定性结束，之后的内容碰撞只可能是新事件，必须透传。tier-(1)（带 seq）的判断逻辑不动（仅在转发带 seq 的 `done` 时同样触发清理，属边界附加而非判定修改）；tier-(2) 的清空在已持锁的 `guard` 上进行（避免 std Mutex 重入死锁）。
  - `sse_dedup.rs`（83 行，纯函数）：`seed_seen`（P0-1 的 overlap 窗口播种）+ `forward_live`（两 tier 判定 + TTL），api.rs 只留接线（755 行）。
- `crates/session/src/runner/drain.rs`
  - `claim_one_queued` 增加 `on_event: &mut (dyn FnMut(SessionEvent) + Send)` 参数；重试仍 Err 时发 `SessionEvent::Error("queued input claim failed: {e2:#}")`，仍返回 None（Empty 语义 → Done，run 正常终止不失败）。调用方 `drain_one_queued` 与 `drain_tests.rs` 两个直接调用点同步改签名。

## 测试清单

| 测试 | 层级 | 断言（红→绿） |
|---|---|---|
| store `inputs_recorded.rs::claim_next_queue_never_reclaims_recorded_rows` | integration（真 libsql 临时库） | 入队 2 条 → claim 第 1 条 → mark recorded=1 → unpromote（恢复 pending）→ 再次 claim 必须拿第 2 条、第 1 条状态保持 (NULL,1)、三次 claim 为 None。红：`left: 1, right: 2`（重新拿到第 1 条） |
| store `inputs_recorded.rs::promote_next_queued_never_repromotes_recorded_rows` | integration | 同构场景走 `promote_next_queued`：再次 promote 必须返回 seq2、recorded 行不被再提升、随后为 None。红：`left: Some(1), right: Some(2)` |
| web `sse_fingerprint_ttl.rs::seen_fingerprints_expire_at_first_forwarded_done`（新文件，HTTP/SSE 层断言） | integration | OverlapStore 播种指纹 X（baseline 前后可观测）→ live `done` 转发 → 内容恰为 X 的 live 事件必须透传（X 恰出现 2 次：replay + live）。红：`left: 1, right: 2`（live 副本被陈旧指纹吞掉；流中可见 `event: done` 已转发） |
| session `queue_claim_error_event.rs::persistent_claim_failure_emits_error_event_and_run_terminates`（新文件） | integration | FailingClaimStore（仅 `claim_next_queue` 恒 Err，其余委托）驱动完整 `run`：必须收到含 "queued input claim failed" 与底层错误文案的 `SessionEvent::Error`，run 返回 Ok、仍有 Done、LLM 恰 1 次调用（搁浅行未消费）。红：事件流只有 `["llm_round_start","text_delta","llm_round_end","done","done","done","done"]`，无任何 Error |
| 回归 | — | store `inputs_recorded` 其余 4 用例、web `sse_overlap_dedup`（BUG 8）/`sse_done_collision`（P0-1）保持绿；session `drain_tests` 两个直接调用点改签名后仍绿 |

- `cargo test -p opencoder-store`：**109 passed / 0 failed**（25 个测试二进制）；`cargo test -p opencoder-web`：**113 passed / 0 failed**；`cargo test -p opencoder-session`：**730 passed / 0 failed**（85 个测试二进制）——均在 75d6866 之后的工作树上实跑。
- `cargo clippy -p opencoder-store -p opencoder-web -p opencoder-session --all-targets -- -D warnings`：**0 警告 / EXIT=0**。
- 行数：`api.rs` 755 / 800（修复直写会到 815 超帽，故拆出 `sse_dedup.rs` 83 行）；`sse_fingerprint_ttl.rs` 279、`queue_claim_error_event.rs` 257（新文件 ≤400）；`inputs.rs` 333、`inputs_recorded.rs` 442、`drain.rs` 354（迭代文件 ≤800）。

## Impact Surface

- **store**：queue drain 语义收紧——`recorded=1` 的行（历史上被 unpromote 恢复 pending 的已消费行）从此对 claim/promote 不可见，杜绝重复入 transcript。影响所有走 `claim_next_queue`/`promote_next_queued` 的调用方（session drain、TUI mirror、web drain watcher 的重启复查）；bulk Steer promote 与 `pending_inputs` 投影不受影响。
- **web SSE**：`GET /v1/session/:id/events` 长流中，首个转发的 live `done` 之后，与 overlap 窗口内容相同的新事件不再被吞（此前整条流生命周期内误删）。对无内容碰撞的流为零行为变化；重复事件在 done 之前仍被正确去重。行为逻辑迁至 `sse_dedup` 模块（纯函数，无对外 API 变化）。
- **session**：queue claim 持续失败从「日志-only 静默搁浅」变为「事件流可见的 Error + run 正常收口」。UI/客户端会多收到一条 `error` 事件（携带失败上下文），run 状态机、Done 语义、重试与 reabsorb 边界均不变。
