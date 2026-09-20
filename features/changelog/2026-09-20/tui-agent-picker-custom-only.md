Commit: 363c826e2d66608bab94689112d51f27efa800a8

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
- 全部 TUI 单元测试：1720 passed / 0 failed；上述三组集成测试在后续工作区
  回归中也全部通过。
- 全量 clippy：`cargo clippy --workspace --all-targets -j 4 -- -D warnings`，通过。
- 修改的 Rust 文件 rustfmt 检查及 `git diff --check`，通过。
- 全量构建：`cargo build --workspace -j 16`，通过；本轮校验设置
  `CARGO_PROFILE_DEV_DEBUG=0`，减少调试符号造成的链接 I/O。
- 全量回归：`cargo test --workspace --no-fail-fast -j 16 -- --test-threads=8`，
  exit 101，5 个测试目标未通过。输出中的完整 `test result` 汇总行累计
  **5426 passed / 3 failed / 7 ignored**；此计数不包含未产生汇总行的中断目标
  和文档测试编译错误。当前未满足仓库全量回归 gate。

全量回归使用独立构建目录 `/data00/rust-build/cargo/myagent-check`，设置
`CARGO_PROFILE_DEV_DEBUG=0`，标准输入为 `/dev/null`。执行期间共享工作区另有
Brain/Control 等模块的变更；上述 clippy、构建结果仅代表各自执行时的代码状态。

### 未通过项

| 测试目标 | 结果 |
| --- | --- |
| `opencoder --test dag_e2e` | `dynamic::runc_dynamic_agent_and_wasm_read_isolated_copies_and_argv` 等待 `dag-dynamic-runc` 终态超过 180 秒。 |
| `opencoder-dag-runtime --test dynamic` | `persistence::scheduling_persistence_failure_cancels_and_drains_live_instances` 和 `recovery::timeout_is_per_instance_and_user_cancel_is_run_cancellation` 返回 `Elapsed`。 |
| `opencoder-agent --bin opencoder-agent` | `host::tests::three_runtime_versions_keep_live_model_calls_and_global_fifo` 持续等待；目标运行 624 秒后以 SIGTERM 中断。单独复核也未在 30 秒上限内结束。栈位于 `dispatch_locked` → `enqueue_capacity` → 临时数据库 WAL 的 `fsync`，具体根因尚未确认。 |
| `opencoder-brain --doc` | E0425：无法解析 `SchedulerPlan` 类型。 |
| `opencoder-control --doc` | E0425：无法解析 `SchedulerPlan`；E0432：无法导入 `opencoder_brain::activation::configured_request`。 |

待处理的是上述运行时超时及文档测试编译问题，解决后需重新取得完整回归通过结果。
本改动未增加忽略测试或放宽断言。

原始结果保留在 `/tmp/opencoder-agent-menu-workspace-tests.log`，检查回执在
`/tmp/opencoder-agent-menu-gates.json`；独立 TUI 单元测试日志为
`/tmp/opencoder-agent-menu-tui-lib-final.log`。

相关记忆：[TUI 模块](../../../agents/tui/index.md)。

本次不包含发布。
