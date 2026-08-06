Commit: (working-tree, pre-initial-commit)

# /config 新增 tmux 开关 + control_helpers 模块抽取

## 背景
两处独立但同属 TUI 输入/配置的改进：
1. `/config` 表单缺少 `enable_tmux_session` 的可视化开关，用户只能手编 config 文件。
2. 控制命令（`/plan`、`/act`、`/act_clear_context`）的输入处理逻辑散落在 `app_helpers`，
   且「是否抑制回显」的判断过宽——`parse_control_cmd(&clean).is_some()` 会把复合命令
   （`/plan fix it`、`/plan $review`）也判为控制命令而不回显，但这类输入携带真实用户内容，
   **应当**进入 transcript。需细化：仅**纯控制命令**（bare，无尾参）抑制回显。

## 变更

### /config 表单新增 tmux 开关
- **`crates/tui/src/model_menu/config_form.rs`**：新增 `ConfigField::EnableTmuxSession`，
  加入 `ORDER`（14 项）；`ConfigForm` 增加 `enable_tmux_session: bool`，`new` 读
  `config.enable_tmux_session.unwrap_or(false)`，`build_patch` 写回 `Some(bool)`；
  `handle_key` 对该字段响应 ←/→/Space 翻转。
- **`crates/tui/src/model_menu/view.rs`**：表单高度 `want_h` 17→18；渲染 `tmux: [ on/off ]` 行。
- **`crates/tui/src/model_menu/patch.rs`**：`ConfigPatch` 增加 `enable_tmux_session: Option<bool>`，
  `to_json` 输出该键（None 时序列化为 null，不强制覆盖）。
- **`crates/core/src/config/merge.rs`**：`has_editable_key` 识别 `enable_tmux_session`，
  使 `/config` 检测到该键时进入可编辑判定。
- **`crates/tui/src/command.rs`**：`/config` 帮助文案追加 `tmux`。

### control_helpers 模块抽取 + 回显判断细化
- **`crates/tui/src/control_helpers.rs`**（新增）：集中控制命令输入处理。
  `forward_skill_if_compound`（从 `app_helpers` 迁入，逻辑不变）+ 新增
  `is_pure_control_cmd`——仅当 `split_control_prefix` 返回 `Some((_, None))`（bare、无尾参）
  才判定为纯控制命令，抑制回显。
- **`crates/tui/src/lib.rs`**：注册 `pub mod control_helpers`。
- **`crates/tui/src/app.rs`**：三处 submit 路径（idle/steer/queue）改用
  `control_helpers::forward_skill_if_compound`；回显抑制判断由内联 `parse_control_cmd.is_some()`
  换为 `is_pure_control_cmd(&clean)`，使复合命令重新进入回显。
- **`crates/tui/src/app_helpers.rs`** / **`app_helpers_tests/mod.rs`**：移除原
  `forward_skill_if_compound` 及 `forward_skill` 测试（迁入 control_helpers）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| tmux 开关读 config true | config_form_tmux_reads_true_from_config | crates/tui/src/model_menu/tests/config_tests.rs |
| tmux 开关 ←/→/Space 翻转 | config_form_tmux_toggles_on_left_right_and_space | crates/tui/src/model_menu/tests/config_tests.rs |
| tmux patch 序列化 true | config_form_tmux_patch_serializes_true_when_on | crates/tui/src/model_menu/tests/config_tests.rs |
| tmux 开关在 config None 时默认 off | config_form_tmux_defaults_off_when_config_none | crates/tui/src/model_menu/tests/config_tests.rs |
| Enter 链路过 tmux 到 Save | enter_chains_through_config_fields_to_save | crates/tui/src/model_menu/tests/config_tests.rs |
| ConfigPatch 含 tmux 字段 | config_patch_serializes_all_fields | crates/tui/src/model_menu/tests/config_tests.rs |
| 表单光标位置随行数偏移更新 | config_form_cursor_on_max_tokens 等 | crates/tui/src/model_menu/tests/{config,cursor_editing}_tests.rs |
| 纯控制命令判定 (bare) | bare_plan_is_pure / bare_act_is_pure / bare_act_clear_context_is_pure / whitespace_padded_bare_plan_is_pure | crates/tui/src/control_helpers_tests/is_pure_control.rs |
| 复合命令非纯（含内容需回显） | plan_with_skill_is_not_pure / plan_with_plain_text_is_not_pure / act_with_skill_is_not_pure 等 | crates/tui/src/control_helpers_tests/is_pure_control.rs |
| forward_skill 复合转发（迁入） | plan_with_skill_forwards_raw / bare_plan_no_skill_keeps_clean 等 | crates/tui/src/control_helpers_tests/forward_skill.rs |

- 全量回归：`cargo test --workspace` → 全绿
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：control_helpers.rs 33 ≤ 400；config_form.rs 迭代中 ≤ 800

## Impact Surface
- `/config` 新增可切换的 tmux 行；既有字段与交互不变。
- 复合控制命令（`/plan fix it`）现在会回显进 transcript（修正：此前被误抑制）。
- 纯控制命令（`/plan`、`/act`）仍不回显，行为不变。
- 不影响：session runner / store / web / cli 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- 同批 echo-text changelog：[queue-steer-consumed-echo-text.md](queue-steer-consumed-echo-text.md)
