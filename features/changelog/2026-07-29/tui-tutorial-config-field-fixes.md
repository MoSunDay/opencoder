# fix(tui/core): 教程自动消失 + /config 数值字段可清空 + 移除 autopilot 死配置

## 背景

三个独立缺陷：

1. **教程弹窗无法关闭** — `app.rs` 在会话启动时计算 `let show_welcome = chat.blocks.is_empty()`
   一次（不可变快照），提交任务后 `blocks` 已非空但快照永远为 `true`；且 `render_welcome`
   是全屏居中 `Clear` 覆盖层，无任何关闭逻辑，教程永久遮挡界面。
2. **/config 数值字段无法清空** — `context_size`/`threshold`/`fps`/`ap_max_iter` 使用原始数字类型，
   Backspace 用 `(v / 10).max(floor)`，永远卡在 floor（1 或 1000），用户无法清空后重新输入。
3. **autopilot skill 死配置** — PLAN 阶段已硬编码 `review` skill（`phases.rs::activate_review_skill`），
   `AutoPilotConfig.skill` 全链路（config → merge → ConfigPatch → ConfigForm）round-trip 但无任何
   运行时读取方，纯属死字段。

## 设计

### Issue 1 — 教程改渲染在 body 区域内

- **删除** 冻结的 `show_welcome` 快照（app.rs）及整条 `show_welcome` 参数链（frame.rs / render.rs）。
- `render_body` 在 `chat.blocks.is_empty()` 时于 body block 的 inner 区域渲染教程文本，
  自动替代空白 transcript。
- `welcome.rs::render_welcome`（全屏 `Clear` + 居中 popup）→ `render_tutorial_in_body(f, inner)`
  （直接在传入 inner 区域渲染 `Paragraph`，左对齐 + 自然换行）。
- 教程文案：去掉虚假的"按 Esc / Enter 关闭"，改为"输入任务后按 Enter 提交，本教程会自动消失"。

效果：提交任何 prompt → blocks 非空 → body 渲染真实内容，教程自动消失，无需按键。

### Issue 2 — 数值字段改 String 缓冲

`threshold` / `context_size` / `fps` / `ap_max_iter` 四个字段从数字类型改为 `String` 缓冲
（与既有的 `max_tokens_input` 完全一致的模式）：

- `new()`：用 `to_string()` 初始化。
- `handle_key` Backspace：`xxx_input.pop()`（可清空到空）。
- `handle_key` Char：`xxx_input.push(c)`（仅 ascii digit）。
- `paste_into()`：同上 push digit。
- `adjust_*`（←/→）：读 String → parse → 增减 → 写回 String（不在输入期 clamp）。
- `validate()`：parse 各 String；空 → 报错；范围校验不变（threshold >= 1000 且 <= context_size）。
- `build_patch()`：parse 各 String；空/不可解析 → 回退安全默认值；fps clamp 1–30。
- `view.rs`：threshold/context_size 行从 String input 派生显示（`≈Nk`），空时显示 `(empty)`。

### Issue 3 — 移除 autopilot skill 死字段

全链路清除（向后兼容：serde 默认忽略未知字段，旧配置含 `"skill"` 不报错）：

- `core/config/autopilot.rs`：删除 `pub skill: Option<String>` 字段、`Default`、`merge()` 分支。
- `tui/model_menu/patch.rs`：删除 `ConfigPatch.ap_skill` 字段及 to_json 的 `"skill"` 键。
- `tui/model_menu/config_form.rs`：删除 `ConfigField::ApSkill` 变体（ORDER 14→13）、
  `ConfigForm.ap_skill_input` 字段、`new()` 初始化、`build_patch()` 分支。
- `session/tests/autopilot.rs`：测试 helper 删除 `skill: None`。

## 测试清单（功能 → 测试名）

| 功能 | 测试 |
|------|------|
| 空会话渲染教程，首条 block 后消失 | `render_tests::empty_session_shows_tutorial_then_hides_on_first_block` |
| Backspace 可清空 threshold 到空 | `config_tests::backspace_clears_threshold_to_empty` |
| 清空后重输替换值 | `config_tests::type_digits_replaces_value` |
| 空字段保存被拦截 + 报错 | `config_tests::save_empty_field_shows_error` |
| threshold/context_size backspace pop | `config_tests::backspace_pops_digit_from_threshold` / `_from_context_size` |
| fps 输入/paste String 缓冲 | `config_tests::typing_digits_sets_fps` / `config_form_paste_into_fps_clamps_at_30` |
| threshold paste String 缓冲 | `config_tests::config_form_paste_into_threshold` |
| context_size 初始化/输入/build | `config_tests::config_form_inits_context_size_from_config` / `typing_digits_sets_context_size` / `config_patch_writes_context_limit` |
| autopilot 初始化/build（无 skill） | `config_tests::config_form_inits_autopilot_from_config` |
| ConfigPatch 序列化（无 skill 键） | `config_tests::config_patch_serializes_all_fields` / `config_patch_omits_max_tokens_when_none` |
| Enter 导航（无 ApSkill） | `config_tests::enter_chains_through_config_fields_to_save` |
| threshold > context_size 拦截 | `config_tests::validate_rejects_threshold_above_context_size` |
| autopilot 配置 round-trip（core） | `config_contract::autopilot_config_roundtrips_through_save` |
| autopilot 循环（session） | `autopilot::*`（13 项全绿） |

## 验证

- `cargo build --workspace` — 零错误
- `cargo clippy --workspace --all-targets -- -D warnings` — 零警告
- `cargo test --workspace` — 1300 passed, 0 failed
