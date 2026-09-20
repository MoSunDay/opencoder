# TUI `/agent` 选择器只展示自定义 Agent

`/agent` 菜单移除内置运行时角色，执行与计划模式继续通过 `/act`、`/plan`
切换。菜单沿用自定义 Agent 目录、名称排序、描述和模糊搜索；选择后仍填入
`/agent <name> `，通过现有提交路径切换并持久化。

- 排除全部内置名称，即使磁盘存在同名注册卡也不展示。
- 没有自定义 Agent 时显示 `no custom agents available`；搜索无结果时显示
  `no matching agent`。空列表按 Enter/Tab 不切换 Agent。
- 底层角色解析、手输控制命令兼容性及 Web 入口保持原有行为。
- 文件系统目录用例移到独立集成测试，菜单单元测试只使用内存数据。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 空目录不补入内置角色 | `empty_registration_has_no_builtin_cards` | `crates/tui/tests/agent_menu_catalog.rs` |
| 自定义卡排序及描述 | `custom_cards_keep_sorted_names_and_existing_descriptions` | 同上 |
| 排除内置同名目录 | `builtin_named_directories_never_enter_the_catalog` | 同上 |
| 空列表确认不切换 | `enter_and_tab_close_empty_results_without_picking` | `crates/tui/src/agent_menu_tests.rs` |
| 空目录与搜索无结果提示 | `popup_distinguishes_no_custom_agents_from_no_search_matches` | 同上 |
| 选择自定义卡并持久化 | `picker_pick_fills_control_head_and_switches` | `crates/tui/tests/agent_mention_flow.rs` |
| `/act`、`/plan` 切换及恢复 | `plan_switch_persists_and_survives_resume`、`act_roundtrip_back_is_persisted`、`switch_never_folds_or_records` | `crates/tui/tests/agent_switch_persist.rs` |

## 验证结果

- 菜单单元测试：`cargo test -p opencoder-tui --lib agent_menu::tests`，8 passed。
- 目录、切换与模式持久化集成测试：`cargo test -p opencoder-tui --test
  agent_menu_catalog --test agent_mention_flow --test agent_switch_persist -j 4`，
  3 + 3 + 3 passed。
- 全量 clippy：`cargo clippy --workspace --all-targets -j 4 -- -D warnings`，通过。
- 修改的 Rust 文件 rustfmt 检查及 `git diff --check`，通过。
- 全量构建：`cargo build --workspace -j 16`，通过；本轮校验设置
  `CARGO_PROFILE_DEV_DEBUG=0`，减少调试符号造成的链接 I/O。
- 首次全量测试在 `brain_e2e` 因独立 target 目录缺少 Server/Agent 运行二进制
  失败；补齐上述 workspace 构建后重跑中，结果待补充。

本次不包含发布。
