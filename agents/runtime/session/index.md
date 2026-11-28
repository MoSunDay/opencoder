Commit: 0abfe0c77

# session 模块

会话运行时：drain 循环、工具注册、subagent、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO。

## 索引
- [src/lib.rs](../../../crates/session/src/lib.rs) — 模块根与公共导出
  （`run`、`run_once`、`run_with_images`、`run_with_registry`、`SessionEvent`）；
  `SessionState::record` 在首条消息持久化时懒创建 session 行（标题取首条用户文本，
  并带上内存中已设置的 `requirement`）；append 外键失败时按内存状态重建已删除的
  会话行并重试一次（`INSERT OR IGNORE`，不恢复级联删除的子行）。
  `SessionState::new` 以 `Some(default())` 初始化模式元数据；Web 新建同样显式写入
  该值，历史数据库行和 bundle 中的整体 `NULL` 不改写为模式感知状态
- [src/runner/](../../../crates/session/src/runner) — drain/执行/sidecar/steer；
  [sidecar.rs](../../../crates/session/src/runner/sidecar.rs) `/sidecar` 子循环：零持久化，
  每轮 usage 以带标签的 `SidecarUsage`（成本通道、可落库，不属于 `is_sidecar_frame`）
  记入主任务 `[tok cost]`，但不写主任务 `real_context_tokens`（ctx 表保持主任务自己的
  最近一轮，见 [changelog](../../../features/changelog/2026-11-28/tui-sidecar-usage-cost-only.md)）；
  [subagent.rs](../../../crates/session/src/runner/subagent.rs) 子代理
  （`subagent` 配置开关默认开启，关闭后隐藏 `task` 工具并拒绝派发、跳过
  恢复重放）、
  [entry/](../../../crates/session/src/runner/entry/mod.rs) 入口 future、
  [drain.rs](../../../crates/session/src/runner/drain.rs) 队列消费
- [src/model.rs](../../../crates/session/src/model.rs) — 按请求解析 Shared/Act/Plan
  有效模型与生成参数；只有精确匹配的内置主 `act`/`plan` agent 使用对应覆盖，
  自定义 agent、build/subagent/workflow 与辅助会话使用 Shared。请求保留完整
  `provider/model` 供 `ChatClient::prepare` 重路由并在 wire 请求中移除 provider
  前缀；有效生成参数和当前模式 `context_limit` 同时供请求、压缩与硬限制检查。
  `take_model_switch` 按完整有效 `provider/model` 去重 `ModelSwitch`：重复调用和仅
  生成参数变化不发事件，同裸模型 ID 的 provider 变化只发一次；`LlmRoundStart` 前
  与 reload/set 共用 `active_model` 同步，避免下一请求重复发事件。
  `inherit_model_overrides[_from]` 供关联新任务继承完整 Shared/Act/Plan 选择。
  Shared 会话选择经 `set_shared_model_override` 持久化并立即成为有效模型，优先于
  `modes.shared.model`；`{}`/缺省字段表示继承配置，整体 `NULL` 保持 legacy。
  显式模型优先级为 `--model` > `OPENCODER_MODEL` > Resume/Fork 会话选择 >
  per-mode config；resume 后重应用同一显式胜者，模式感知行清空 Shared/Act/Plan
  为 `{}`，legacy 行保持 `NULL`，同时保留分模式生成参数与 context overlay。
  Env reload 持久化目标 Env 的 Shared 到 `sessions.model` 并清空全部会话选择为
  `{}`，避免 resume 回到旧 Env。两条路径均传播 Store 错误，`sessions.model` 成功而
  元数据写失败时补偿第一步，显式路径还恢复 config、模型标签与请求身份。
  新会话保持首次输入才落行；启动显式覆盖和关联任务继承只改内存。会话模型操作需要
  物化懒行时，以最终模型/元数据一次插入，后续 harness 写失败只回滚确认由本次新建的
  行，已有行只更新且绝不作为补偿删除对象；回滚失败会明确报告残留。Env reload 保留
  `skill_body_delivered`，不重复投递已消费的一次性 Skill body
- [src/subagent_steer_gate.rs](../../../crates/session/src/subagent_steer_gate.rs)
  — 非中断 subagent steer 的运行时准入门，锚定 provider-turn 边界；
  `lib.rs` re-export `SteerReservation`/`SubagentSteerGate`
- [src/harness/](../../../crates/session/src/harness) — 前端共用的执行器准备与
  续会话校验（`SessionState` 入口）
- [codex/launch.rs](../../../crates/session/src/harness/codex/launch.rs)、
  [process.rs](../../../crates/session/src/harness/codex/process.rs) — Codex
  进程启动与管理
- [codex/control/](../../../crates/session/src/harness/codex/control/mod.rs)
  — 可选外部输入协议
- [harness/output.rs](../../../crates/session/src/harness/output.rs) — 共享模型
  最终文本的 JSON 提取
- [src/tools/](../../../crates/session/src/tools) — 工具注册与实现；
  [command/](../../../crates/session/src/tools/command/mod.rs) 共享命令执行与
  后台任务
- [src/bash_guard.rs](../../../crates/session/src/bash_guard.rs) — plan/sidecar
  只读执行门，Bash 判定走 [shellguard](../shellguard/index.md)
- [src/compaction/](../../../crates/session/src/compaction) — 上下文压缩；
  `SUMMARY_PREFIX` 是合成摘要消息的公开标记（前端回放据此渲染压缩内容）
- [src/prompt.rs](../../../crates/session/src/prompt.rs)、
  [prompt/operator.rs](../../../crates/session/src/prompt/operator.rs) — 系统提示
  构建与 Operator 规则
- [skill_resolve.rs](../../../crates/session/src/skill_resolve.rs) —
  技能与 `@` 提及解析
- [src/resume.rs](../../../crates/session/src/resume.rs) — 会话恢复；
  [src/fork.rs](../../../crates/session/src/fork.rs) — 会话派生；两者保留完整
  Shared/Act/Plan 可选覆盖及整体 `NULL`。legacy 行首次转为模式感知元数据时，会为
  未选择的模式显式保留原 legacy 模型，不追溯套用新的模式配置覆盖
- [src/process/](../../../crates/session/src/process) — 进程身份与平台进程树
  （[identity.rs](../../../crates/session/src/process/identity.rs)、
  [windows.rs](../../../crates/session/src/process/windows.rs)）
- [tests/](../../../crates/session/tests) — 集成回归；支持边界见
  [Windows](../../../features/platform/windows/index.md)
