Commit: (working-tree, pre-initial-commit)

# fix(session): queued 一次只提交一条真实 prompt，steer 才是清空路径

## 背景

两段式 delivery 的文档契约（`features/index.md`、`agents/session/index.md`）一直是
「**idle 时消费恰好一条 queue**；steer 在 turn 边界吸收全部 pending」。但
`run_loop` 的 idle 排空用 while 循环持续 `claim_one_queued`：真实 prompt 也会一个接
一个跑 LLM turn，直到队列清空才发 `Done`——代码越权，队列一次提交了多条。

## 变更

`crates/session/src/runner/mod.rs::run_loop` 新增局部标志 `queue_real_consumed`：

- idle 边界先检查该标志：已消费过真实 prompt → 直接发 `SessionEvent::Done` 并
  break，剩余队列行保持 pending（待下次显式提交）。
- 未消费时照旧排空：控制命令（/act、/plan、/act_clear_context）连续应用不消耗
  LLM turn；真实 prompt 记录后置位 `queue_real_consumed` 并 break，外循环跑这一个
  turn，下一 idle 边界收尾。**每 run 恰好一个 queued 真实 prompt**。
- steer 语义不变：`claim_steers` 仍每轮 turn 顶部提升全部 pending `Delivery::Steer`
  并发 `SteerConsumed{seq}`——清空/一次多条只走 steer。
- 存储层无需改动：`claim_next_queue` 本就是 LIMIT 1 原子领取，问题只在 runner 循环。

TUI 侧（`crates/tui/src/app_loop.rs::fold_ui_events`）：`Done` 不再等价于
「队列必空」，因此不再 `queue_items.clear()`，改为经
`queue_panel::pending_mirror(store.pending_inputs(...))` 从 store 重同步镜像，已消费
行之后的剩余行继续显示在侧边面板。

## 测试

- `tests/steer_followup.rs`：`queue_only_promotes_at_idle_exactly_one_per_cycle` 重写为
  `queue_promotes_exactly_one_real_prompt_per_run_then_stops`——run 1 只消费 QUEUE-1
  （断言 QUEUE-2 仍 pending），run 2 显式提交消费 QUEUE-2，队列最终排空。
- `tests/control_cmd.rs`：`queue_drains_control_cmds_between_real_prompts` 更新为
  「/plan 无 LLM 应用、do work 跑一个 turn、尾部 /act 保持 pending」——最终
  agent=plan、consumed_count==2、pending 恰含 /act。

## 影响面

- 会话运行时 queue 消费节奏（每 run 一条，剩余等待下次显式提交）。
- TUI 侧边队列面板在 run 结束后仍显示剩余 pending 行（不再被 Done 清空）。
- web drain / CLI headless：不受影响（steer 与显式提交路径语义未变）。

## 相关文档

- [agents/session](../agents/session/index.md)（run_loop idle 排空描述已修正）
- [features/index.md](../index.md)（既有「idle 时消费恰好一条 queue」契约现已由代码兑现）
