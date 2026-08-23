Commit: a1b22a1a52fc7538bdb5c79f8eca45f03cdd6642

# 状态栏任务计时器状态着色：运行中橙色 / 停止后灰色

## 背景与根因

状态栏的累计任务计时（`task_ms`）在任务运行期间以 warn（橙/黄）色渲染，且任务停止后 `task_elapsed_ms` 冻结但**颜色不变**——橙色持续到下一次提交需求，无法一眼区分“正在跑”和“已停止”。底部动画（braille spinner、状态点闪烁）在停止时已消失（既有行为），但计时器仍保持运行色，与动画的状态信号不一致。

## 新稳定契约

- 任务运行中（`running == true`）：累计计时 `[warn_color]` 橙色——与 spinner 同色。
- 任务停止后（`running == false`）：累计计时保持可见（冻结值），但颜色切换为 `[muted]` 灰色；spinner 消失。
- 继续提交需求（`running` 重新置真，`task_elapsed_ms` 归零重新累计）：计时自动回到橙色。
- 状态区分完全由 `render_status` 的 `running` 参数驱动，无新增状态字段；与 subagent 表头计时器（`push_duration_span`：live warn / done muted）采用同一配色对。
- body 底边框 `[turn cost]` 计时不受影响：`display_tail_ms` 在停止时返回 0，该计时本就隐藏。

## Validation

- TUI：新增 `status_bar_task_time_turns_muted_when_stopped`（停止后 `1m30s` 仍可见、颜色为 muted、无 spinner 字符）；既有 `status_bar_shows_task_time`（运行中 warn 色 + 位于 spinner 前）不变通过。
- `cargo test -p opencoder-tui --lib`：1513 passed / 0 failed；`cargo clippy -p opencoder-tui --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check` 通过。

## 兼容性与边界

- 无数据库 schema、配置项或环境变量变化；无公共 API 签名变化（`render_status` 签名未动）。
- 停止后计时仍显示冻结值（灰色），不隐藏——用户可回看本任务总耗时。
