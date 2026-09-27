Commit: b03836fdcce953baa47f6d20993811cc15cdf35b

# 大脑运行详情抽屉与人工输入

schema 7 运行页主视图只展示轮次、层级状态、画布和暂停、取消、查看详情操作。右侧抽屉按轮次展开表格，每条执行一行；点击执行 ID 在同一抽屉中打开通用能力详情。

计划详情和 Agent/Operator 执行详情的人工输入统一写入大脑的 `human_input` 事件，不指定子执行 ID，也不直接发送子执行命令。运行中的大脑可先记录 `guide`，等待层屏障后再决定调度。调度上下文读取全部人工输入与已处理引导；超出上下文容量时明确报错。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 主视图、轮次表格、执行详情与返回 | `run.dom.test.jsx` | `crates/web/spa/src/brain/workbench/milestone/` |
| 两种入口只发送大脑输入、刷新失败提示 | `run.dom.test.jsx`、`embeds.dom.test.jsx` | `crates/web/spa/src/` |
| 输入接口拒绝目标 ID、空输入和终态输入 | `human_input_is_recorded_only_as_a_brain_event` | `crates/control/tests/e2e/layered_api/commands.rs` |
| 层屏障期间的引导与决策恢复 | `brain_scheduler_v4`、`milestone` | `crates/worker/tests/`、`crates/brain/tests/` |
| schema 7 进程级计划运行 | `brain_layered_e2e` | `tests/brain_layered_e2e/` |
| 真实页面结构与多能力执行记录 | `runtime.js`、`layered-panels.js` | `scripts/acceptance/brain/` |
