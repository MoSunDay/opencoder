# TUI 三个用户可感知缺陷修复：中断无效 / 双 Shift+Tab 不立即执行 / 标题模型名与实际不符

一句话：为 UI 回合引入**回合代数（turn generation）**——`ResetCancel`
携带代数、`TurnDone` 回带代数、`AdmitReq`/`AdmitDone` 透传代数，主循环
按代数丢弃过期完成事件；中断判定从 `running` 放宽为
`running || subagents_running`；标题栏模型名改为镜像**会话实际模型**
（`chat.model`），外层 config 模型仅作缺省回退。

## 缺陷与根因

1. **有概率中断无效**
   - 子代理排空窗口（`running=false` 且 `chat.subagents_running>0`）里，
     Ctrl+C 走 `running` 单一门槛 → `KeyAction::Quit`；双 Esc 直接落空。
   - 取消后，过期 `SessionEvent::Done` 消费掉唯一的 `cancelled=true`，
     迟到的 `AdmitDone`（idle_rekick）会把用户刚取消的排空工作“复活”。
2. **双 Shift+Tab（act 清场确认）不立刻执行**
   - 手动确认 `fire_clear_confirm_now` 只在 `*running` 时取消；排空窗口
     下不清场而是排队。
   - worker 串行处理 UiCmd：被取消回合的过期 `TurnDone("plan")` 抢先落
     else 分支 → `running=false`、`chat.agent="plan"`，而 act 回合仍在
     队列里，UI 看起来“没执行”。
3. **标题模型名与实际模型不符**
   - 标题渲染外层 `run_app` config 的 `config.model_id()`；会话切换 /
     模型切换后的实时 `sess.config.model`（持久化于 `sessions.model`）
     从不显示。

## 行为口径

- `UiCmd::ResetCancel(CancellationToken, u64)` / `UiEvent::TurnDone(String, u64)`：
  worker 循环记录最近一次 `ResetCancel` 的代数并在 `TurnDone` 回带；
  `start_turn` 与 `cancel_running_turn` 各自递增代数；会话切换重置为 0
  （新 worker 从 0 起）。`fold_ui_events` 丢弃代数不匹配的 `TurnDone`
  （不翻 `running`、不改 agent、不再触发排空重启）。
- `AdmitReq`/`AdmitDone` 携带 `turn`；`on_admit_done` 仅在
  `done.turn == *turn_gen` 时允许重踢（排队行落账不受影响）。
- 中断门槛统一 `running || chat.subagents_running > 0`：
  `key_handler` 的 Ctrl+C 与双 Esc、`fire_clear_confirm_now` 的手动确认。
- 标题：`chat.model` 非空时取其 `/` 后段显示；空则回退
  `config.model_id()`。同步点：`SessionEvent::ModelSwitch`、
  `/model` 菜单保存、会话切换装载（switch / resume / session-only）。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| 过期 `TurnDone` 整体丢弃（不消费排空臂、不翻 idle、不改 agent、不重踢） | `app_loop_tests/stale_turn_done_tests.rs::stale_turn_done_is_dropped_entirely` |
| 代数匹配的 `TurnDone` 照常处理（守门只拦过期） | `…::matching_turn_done_still_processes` |
| 过期代数 `AdmitDone` 不重踢排空（镜像仍落账） | `idle_rekick.rs::stale_turn_admit_done_does_not_rekick` |
| Ctrl+C 排空窗口内 Cancel 而非 Quit；全空闲仍 Quit | `key_handler_cancel_busy_tests.rs::ctrl_c_during_subagent_drain_cancels_instead_of_quitting` / `ctrl_c_when_fully_idle_still_quits` |
| 双 Esc 排空窗口内硬中断；全空闲保持输入清除语义 | `…::double_esc_during_subagent_drain_cancels` / `double_esc_when_fully_idle_stays_input_clear` |
| 手动确认在排空窗口（parent idle + 子代理未归）打断并立即开清场回合 | `app_loop_dispatch_cmd_tests/act_clear_busy.rs::manual_confirm_during_subagent_drain_fires_now` |
| 标题镜像会话实际模型（外层 config 不渗漏） | `app_loop_tests/display_title_tests.rs::compute_display_title_prefers_live_session_model_over_config` |

既有测试的机械适配（签名加 turn/turn_gen 形参）覆盖：worker
process_cmd、queue/steer admit、subtask 追加、fold_ui_events、
dispatch_slash/mode_switch、clear-confirm 全链。
