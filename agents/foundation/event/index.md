Commit: 573bb2e48fa20832a47b7bec8adc8ee3c08e8e78

# event 模块

`opencoder-event`：事件驱动调度的通用基建。默认纯内存、零存储引擎
依赖；不附带任何持久化后端。尚未接入任何生产
dispatcher，基础存储（turso 的 [store](../store/index.md)）不受本 crate
影响。

## 索引
- [src/domain/](../../../crates/event/src/domain) — 纯域：事件词汇与不可变
  记录（`event.rs`，开放 `namespace.name` 词汇 + `event.custom.<x>` 扩展）、
  监听器 spec 与匹配（`listener.rs`、`registry.rs`）、消费者 id/游标校验
  （`consumer.rs`）、全局暂停门（`gate.rs`，暂停只拦截分发、不拦截日志追加）
- [src/ledger.rs](../../../crates/event/src/ledger.rs) — `EventLedger` 持久化
  契约（日志追加/严格游标读、监听器 CRUD、暂停等待登记、消费者游标）与
  默认实现 `MemoryLedger`（参考语义，测试用）
- [src/bus.rs](../../../crates/event/src/bus.rs) — 广播总线：broadcast hub
  扇出，不保留重放历史（持久重放归 ledger）；慢订阅者 Lagged 不阻塞快订阅者
- [src/executor.rs](../../../crates/event/src/executor.rs) — 触发规划与执行：
  `plan` 纯函数 + `dispatch`/`dispatch_gated`，逐监听器上报结果；门控结果
  `DispatchDecision::{Paused, Dispatched}` 可区分暂停与空分发
- [src/consumer.rs](../../../crates/event/src/consumer.rs) — 消费者事件泵
  `run_pump`：严格按持久游标重放，整批动作全部成功才前移游标；失败保留
  游标并上报 seq + 监听器 id（at-least-once）；门暂停返回 `Paused` 且不动
  游标

## 测试
- [tests/bus_ledger.rs](../../../crates/event/tests/bus_ledger.rs) — 总线
  扇出与 `MemoryLedger` 语义（游标读、监听器 CRUD、等待匹配）
- [tests/consumer_pump.rs](../../../crates/event/tests/consumer_pump.rs) —
  游标重放、失败保留后重试推进、分批 chunk、暂停恢复
- [tests/executor.rs](../../../crates/event/tests/executor.rs) — 规划选择与
  派发逐监听器结果
- [tests/registry.rs](../../../crates/event/tests/registry.rs) — 监听器注册
  校验与匹配顺序
- 纯函数内联测试在 `src/domain/*`、`src/ledger.rs`。

## 生产边界

未接生产：无生产 `ActionExecutor`、outbox 或控制面接线，没有任何调度
循环调用 `run_pump` / `dispatch_gated`；`SchedulerGate` 无生产 setter
（默认放行）。`execution.paused`、`brain.*` 为保留词汇，当前无 producer。

## 相关
- [store](../store/index.md) — 基础存储（turso）；事件队列在 Store 之外
