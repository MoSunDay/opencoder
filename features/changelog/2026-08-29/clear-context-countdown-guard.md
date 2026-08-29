Commit: (working-tree, /act_clear_context canonical 更名 + 倒计时确认防护)

# /act_clear_context canonical 更名 + Shift+Tab 倒计时确认防护（误操作防上下文丢失）

## 背景

clear-context 折叠是全系统唯一「丢上下文」的操作：一次误触 Shift+Tab（或误发命令）就把除最后回复外的全部 transcript 折叠掉，不可逆。本轮把它从「gate-and-go 直发」改为「倒计时确认 + Esc 回撤」，并把 canonical 命名改回 `/act_clear_context`（act 前缀显式化——它就是 act 代理的折叠重启动作；纠正 33b1ee2 的 canonical 记录方向，`/clear_context` 降为 legacy 别名，持久化输入仍可解析）。

## 实现

- **tui 新模块 `clear_confirm.rs`**（≤400 行，纯函数 + 单测）：
  - `ClearConfirm { armed_at, rest, restore_draft }`；`CLEAR_CONFIRM_WINDOW_MS = 5s`；`CLEAR_CONTEXT_CMD = "/act_clear_context"`（canonical，单一真相源）。
  - `head_rest`：两条拼写（含复合尾部）的头部分词判定——修复 idle 下键入复合命令经 `command::parse` 丢尾部、原文泄漏给模型作为普通 prompt 的存量缺陷；`/act_clear_contextx` 不误匹配。
  - `engage`：arm + transcript 回显保留的最后 assistant 回复（`chat.last_reply_text()`，ChatBlock 侧预览镜像 runner 侧 `handoff::last_assistant_text`）+ 取消提示；`intercept`：armed 期间吞掉全部按键（Enter = Fire、Esc = Cancel 并恢复草稿入输入框、其余 inert）；`tick`：窗口到点返回 arm 由调用方 fire；`banner`/`refresh_flash` 借道 mode-flash chip 每动画 tick 输出秒数（`→ clear` 前缀 → `frame::is_warn_flash` warn 黄判别（sandbox/plan/clear 三族，render 消费 + 具名单测））。
  - fire 路径 `app_loop_actions::fire_clear_confirm`：idle 直接 `UiCmd::Prompt("/act_clear_context [+rest]")` 开 turn（镜像 mode-switch Run 臂），running 经 queue 原文入队由 runner 在 idle 边界应用；armed 键拦截/tick 派发收敛为 `handle_confirm_key`/`confirm_tick`，app.rs 净增 31 行（799/800）。
- **触发路径统一**：Shift+Tab（`KeyAction::ArmClearConfirm`，含草稿复合尾）、idle 键入命令（`maybe_arm` 先于 `command::parse`）、popup Enter（`dispatch_slash_action` ClearContext 臂 arm 而非 gate）、运行中键入命令（Steer 臂 arm，fire 改走 queue）。`ModeSwitch::ClearContext` 变体删除（不再有 gate 直发路径）。
- **session**：`control_cmd.rs` canonical 换序 `/act_clear_context` | `/clear_context`（解析行为不变，两拼写同效）；注释/文档同步。

## 测试

- `clear_confirm` 8 例（head_rest 边界/compound、command_text、remaining/expiry、intercept Enter/Esc/inert、maybe_arm、preview 截断）。
- `act_clear.rs` 重写：popup dispatch arm（idle + running）、fire idle 提交 canonical 复合 prompt、fire running 入队、`backtab_and_typed_clear_context_are_one_path`（canonical 回程 + head_rest）。
- `key_handler_running_mode_tests::backtab_arms_clear_context_confirm`（ArmClearConfirm + 草稿转发 + 输入清空）。
- `frame::tests::warn_flash_hue_covers_sandbox_plan_and_clear_guard`（warn 黄判定提为 `frame::is_warn_flash` 纯函数后补齐的专属测试）。
- `switch_gate_tests` 收敛为 Act/Sandbox 两模式；`command.rs` canonical 断言换序；session `control_cmd` 注释/文档换向。
- 全量回归：`cargo test --workspace` → 3322 passed / 0 failed（233 个测试二进制，TEST-EXIT=0，当次实跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（CLIPPY-EXIT=0）

## 回修（同日）：Esc 回撤后倒计时 chip 残留

- **现象**：armed 期间 Esc 回撤，「[clear] 已取消（回撤）」marker 正常落盘，但右下角倒计时 chip（`→ clear Ns 后清空上下文 · Enter 立即 / Esc 取消`）常驻不消失。
- **根因**：chip 借道 mode-flash 槽位渲染，`clear_confirm::tick` 每动画 tick 刷新以活过 15-tick flash 生命期（设计如此，否则秒数跳不到 0）；Esc 取消臂只推 marker 未清 `mode_flash`，而 guard 拆除后 idle 下 `anim_tick` 冻结，`frame::flash_visible(start, now, 15)` 用冻结 now 判定 flash 永远可见。
- **修复**：`app_loop_actions::handle_confirm_key` 的 `ConfirmFlow::Cancel` 臂在 `push_cancel_marker` 后补 `*mode_flash = None;`（语句级，零签名变更）；Fire（idle 提交 / running 入队）与到点自动 fire 路径不变，running 路径 flash 仍随 running tick 自增自然过期。
- **测试**：新增 `act_clear::esc_cancel_drops_countdown_chip`（engage 抬 chip 并吞草稿 → Esc → 断言 `mode_flash`/`clear_confirm` 同步清空、回撤 marker 落盘、`restore_draft` 回填输入框、不发任何命令）；既有 act_clear 5 例 + clear_confirm 8 例不降。
- 全量回归：`cargo test --workspace` → 3323 passed / 0 failed（233 个测试二进制，当次实跑；基线 3322 + 新 1 例）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；`cargo build --workspace` 干净
