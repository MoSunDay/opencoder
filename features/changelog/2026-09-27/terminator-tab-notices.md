Commit: b8d46650b4c6c349d5c5dc6b276f11670fff7b87

# TUI 完成与提问 Hook

TUI 在父回合进入最终空闲状态时触发 `turn_done`，在 `question` 工具开始等待回答时触发
`question`。命令从 `~/.opencoder/hooks.json` 读取，以 `sh -c` 异步执行；每条最多等待
3 秒，失败只记录 debug 日志。配置可调用终端的 `terminator-ctl notice`，让非活动 tab
显示待介入提示。事件读取配置、执行命令和缺失配置的行为由 `crates/tui/src/hooks.rs`
测试覆盖。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 事件命令选择和执行 | `configured_events_select_commands_and_execute` | `crates/tui/src/hooks.rs` |
| 缺失与错误配置 | `missing_config_has_no_hooks_and_invalid_json_is_reported` | `crates/tui/src/hooks.rs` |
| 提问事件读取当前配置并执行 | `emitted_question_uses_the_current_config_home` | `crates/tui/src/hooks.rs` |

- TUI 单测：`cargo test -p opencoder-tui --lib` → 1728 passed / 0 failed。
- Clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 原全量回归暴露的 `brain_layered_e2e` 旧 schema 6 用例已更新到 schema 7，定向复测 5/5 通过；完整工作区回归仍在执行。
