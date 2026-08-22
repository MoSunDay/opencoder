Commit: 52622d77bf761b10cf72f97ced5100e2c93961f9

# running 模式切换门回归加固

## 背景与根因

产品不变量是：会话处于 running/draining 时，用户不能把 act/plan 切到另一模式；切换必须立即拒绝，由用户在 idle 后重试，不能延迟排队。此前只覆盖了直接切换键和部分 HTTP agent/model 入口，仍存在三类绕过：

- TUI 的 Enter/Tab 自由文本和聚焦 subagent 输入可把复合 `/plan ...`、`/act ...` 当普通 steer/queue admit；BackTab 也有复合 `/plan` 分支在切换门之前清空输入。
- Web prompt body、文本模式命令、handoff、subagent steer 没有共享同一 admission 判定；前端下拉还会在服务端确认前乐观提交 mode。
- Web 的“检查 draining → 写 meta/override/admit → 启 drain”分散在多个锁域，存在并发 TOCTOU；最后 SSE subscriber 淘汰并替换 handle、drain 尾部过早清 running 又会产生第二个 lifecycle 域或假 idle 窗口。

根因不是单一漏判，而是缺少“模式控制分类 + per-session lifecycle 串行化”这两个全局不变量。只在每个 handler 补一次布尔检查，无法封闭并发和新入口。

## 稳定契约

- `running/draining == true` 时，所有用户发起的 act/plan transition 都立即拒绝；包括裸/复合 `/act`、`/plan`、`/act_clear_context`、agent override 和 handoff。
- 拒绝是无副作用的：不清输入，不写 skill/input/message/meta/override，不发送 drain 命令，不安排 idle 自动补发。
- TUI 显示既有 `⏳ busy — mode switch blocked, retry when idle`；Web 返回 409。普通 prompt 的 steer/queue 与运行中 compact 保持可用。
- runner 的控制命令 parser 仍处理 idle 输入和历史已持久化/内部恢复数据，只是 defense-in-depth，不代表公共入口可以运行中排入模式命令。

## 实现

### 共享分类与 TUI

- `crates/session/src/control_cmd.rs` 导出纯函数 `is_mode_control`，统一识别三类裸命令和携带内容的复合命令。
- `crates/tui/src/key_handler.rs` 在任何输入清空、history/skill/store admission 之前裁决 Enter、Tab、复合 BackTab 与 subagent-focus 输入；命中时返回 `ModeSwitchBlocked`，app 只显示统一 busy flash。
- Shift+Tab 等直接模式键继续经过 `app_loop::handle_switch_agent`；worker 的 running gate 作为最后防线保留。

### Web lifecycle 与 API

- `SessionHandle` 新增 per-session lifecycle mutex；`lock_session_lifecycle` 在取得锁后复核 HandleMap 的 Arc 身份，handle 被替换则重试。最后 SSE subscriber 的 eviction 也在此锁内，避免产生并行旧/新 handle。
- 所有 false→true drain 启动集中到 `start_drain_locked`。`admit_and_drain_guarded` 在 lifecycle 临界区内完成 mode busy 裁决、可选 skill 持久化、input admission 和 drain 启动。
- `DrainGuard` 最后才清 `draining`：先等待事件 flusher、恢复 cmd receiver，再暴露 idle，关闭 drain 尾部假空闲窗口。
- POST prompt（body agent 或模式文本）、agent、model、handoff 与 subagent steer 统一在副作用前拒绝。agent 切 plan 时用一个 `SessionPatch` 同时写 agent 与 `plan_input_count=0`，不再依赖命令队列或失败后的竞态回滚。

### Web 前端

- agent selector 分离“已提交 mode”和“待切 mode”；busy/pending 时禁用 agent 与 handoff，服务端 409 恢复已提交值，成功响应后才 commit。
- composer 在 busy 时本地拒绝三类裸/复合模式命令，保留文字、skill 与图片且不发请求；服务端仍做权威二次校验。

## 受影响范围验证

- session control command 单元过滤：20 passed。
- TUI `key_handler_running_mode_tests`：5 passed；既有 `switch_blocked_while_running`：2 passed。
- Web lib：17 passed；`running_mode_gate`：1 passed；`agent_model_toctou`：2 passed；`subagent_steer_api`：7 passed；frontend runtime：1 passed。
- `frontend_smoke.mjs` 全部 running-mode 场景通过。
- 根目录真实二进制 `running_mode_switch_e2e`：1 passed，覆盖 `opencoder serve`、阻塞 provider、HTTP 409、无持久化副作用和 idle 后成功切换。
- 受影响 crate/测试目标的 clippy `-D warnings` 通过；仅对 Web 中两处既有 `clippy::result_large_err` 做定向 lint 豁免。`cargo fmt --all -- --check` 与 `git diff --check` 通过。

按本次回归范围只运行上述受影响测试，未执行 workspace 全量测试。

## 兼容性与边界

- 无数据库 schema、配置项或环境变量变化。
- 普通运行中 steer/queue、interrupt、compact 语义不变；handoff 因包含模式切换只能 idle 执行。
- 运行中拒绝不会消费用户输入或激活 skill；idle 后由用户明确重试。

## 相关文档

- [session 模块](../../../agents/session/index.md)
- [TUI 模块](../../../agents/tui/index.md)
- [Web 模块](../../../agents/web/index.md)
- [模式切换 running-gate 原始契约](../2026-08-08/mode-switch-running-gate.md)
- [双向拦截校准](../2026-08-19/plan-switch-direction-aware-running-gate.md)
