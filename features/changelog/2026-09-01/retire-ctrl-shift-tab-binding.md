Commit: (working-tree)

# 退役 ctrl+shift+tab 绑定与描述

## 背景

`switch_mode`（Ctrl+T）恢复后，`switch_mode_keep`（ctrl+shift+tab）已是冗余的
第三个模式切换键；帮助页与 `parse_key_spec` 文档示例仍在宣传它。本轮把该绑定与
所有面向用户的描述清除，act/plan 切换收敛为：Ctrl+T（双向）+ Shift+Tab
（模式感知，见 act-shift-tab-mode-aware-switch）。

## 变更摘要

- 两份 keymap 无 `ctrl+shift+tab` / `switch_mode_keep` 绑定（`grep` 0 命中）；
  `parse_key_spec` 文档示例改用 `"ctrl+t"`。
- core 的 legacy 兼容守卫测试改以 alt+tab 为退役字段代表
  （`switch_mode_keep` 从 fixture JSON 移除）；普通 `Deserialize` 忽略未知字段
  的保证不变，旧配置携带 `switch_mode_keep` 仍被静默忽略。
- 帮助页负向断言守卫 `!HELP.contains("Ctrl+Shift+Tab")` 继续把守描述面。
- **有意保留**：`parse_key_spec` 的两条通用解析测试
  （`parse_ctrl_shift_tab_normalizes_to_backtab`、`match_ctrl_shift_tab`）——
  解析器仍是通用配置面，`"ctrl+shift+tab"` 作为输入串仍可解析成键组合，
  只是不再是任何功能的默认/示例绑定。历史 changelog 不回写。

## 兼容性

无 schema 变更：未知 keymap 字段继续被忽略（向后兼容老用户配置）；
守卫强度从"两个代表键"降为"一个代表键"（alt+tab），能力覆盖与痕迹清零的
显式权衡。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| legacy 配置忽略退役字段、恢复 `switch_mode` | `legacy_keymap_restores_switch_mode_and_ignores_retired_variants` | `crates/core/src/config/keymap.rs` |
| 帮助页无 Ctrl+Shift+Tab / Alt+Tab | `help_matches_plan_world`、`help_no_stale_hide_composer_shortcut` | `crates/tui/src/keymap_menu/help.rs` |
| 解析器通用能力（有意保留） | `parse_ctrl_shift_tab_normalizes_to_backtab`、`match_ctrl_shift_tab` | `crates/tui/src/keymap_tests.rs` |

## 全量回归

- `cargo test --workspace` → @REGRESSION@（收敛树上采集）

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [agents/core](../../../agents/core/index.md)
