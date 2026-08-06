Commit: (working-tree, pre-initial-commit)

# 修复 /config 保存时 null patch 静默删除 reasoning_effort 等键

## 背景
TUI `/config` 表单保存时，`ConfigPatch::to_json()` 对 `None` 的 `Option` 字段
（`reasoning_effort`、`interleaved_thinking`、`enable_tmux_session`）发出显式 `null`。
而核心层 `merge_json`（RFC 7396 JSON Merge Patch 语义）将 `null` 解释为「删除该键」。
两者叠加导致：用户把 reasoning 切到 **Off** 再保存后，`reasoning_effort` 键被从磁盘
JSON 文件中**永久删除**——下次 `load()` 读不到该键走默认 `None`，表现为 reasoning
莫名回 off。`max_tokens` 因有 `if let Some` 守卫而幸免，其余字段无此保护。

## 变更

### ConfigPatch 序列化修正
- **`crates/tui/src/model_menu/patch.rs`**（`to_json`）：
  - `reasoning_effort` 为 `None`（Off）时改为发出 `""`（空字符串）而非 `null`——键得以
    在磁盘持久化；`merge_into` 已正确将空串映射为 `None`（runtime 行为不变）。
  - `interleaved_thinking` / `enable_tmux_session` 改用 `max_tokens` 同款 `if let Some`
    守卫——为 `None` 时**省略**该键（不覆盖磁盘既有值）。
- **`crates/tui/src/model_menu/config_form.rs`**：修正 `Reasoning` 枚举误导性文档注释
  （原称「Off serializes to null」，现改为「serializes to ""」）。

### 设计说明
核心层 `merge_json` 的 `null = delete` 语义**未改动**——这是 Web API `PATCH /api/config`
显式删除键所需的（RFC 7396 合约，由 `save_can_remove_reasoning_effort_via_null` 契约
测试锁定）。仅 TUI 表单层停止用 `null` 作为「Off」表示。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| Off 时 reasoning 输出 "" 而非 null | config_patch_off_reasoning_emits_empty_string_not_null | crates/tui/src/model_menu/tests/config_tests.rs |
| enable_tmux_session 为 None 时省略键 | config_patch_serializes_all_fields（更新断言） | crates/tui/src/model_menu/tests/config_tests.rs |
| 空串 reasoning_effort 解析为 None | reasoning_effort_empty_string_resolves_to_none | crates/core/tests/config_contract.rs |

- 全量回归：`cargo test --workspace` → 全绿
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：patch.rs 85 ≤ 400；config_form.rs 372 ≤ 800；config_tests.rs 716 ≤ 800；config_contract.rs 767 ≤ 800

## Impact Surface
- `/config` 表单保存 Off reasoning 不再删除磁盘键；既有值在下次加载时正确保留。
- 不影响：核心 `merge_json` / Web API `PATCH /api/config` 的 `null = delete` 合约；
  CLI / session / store / web 边界不受影响。

## Related Docs
- [agents/core](../../agents/core/index.md)
- [agents/tui](../../agents/tui/index.md)
- 同区 config 拆分：[config-rs-split-under-line-gate.md](config-rs-split-under-line-gate.md)
