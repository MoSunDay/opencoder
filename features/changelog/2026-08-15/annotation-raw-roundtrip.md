# /annotation 逐字节原样保存与会话切换恢复

## Context

`/ann` 提交的文本在 UI 缓存链路被 `sanitize_multiline` 改写（TAB→4 空格、删控制字符），再编辑再保存会用改写版覆盖 DB 中的原始提交，往返丢失；`/task` 切换会话后 `annotation_text` 不从 `requirement` 恢复，重开 `/ann` 会用 `first_prompt` 顶替原内容落库。空保存存 `Some("")` 同样触发 first_prompt 回退覆盖。

## Change Summary

- `chat_req.rs`：`update_annotation_text` 删除 sanitize，UI 缓存与 DB `sessions.requirement` 逐字节一致。
- `app_task.rs`：`/task` 切换后 `chat.annotation_text` 从持久化 `requirement` 恢复（与 `app.rs` 启动路径对称），持久值优先于 UI 快照。
- `store`：`SessionPatch` 新增 `clear_requirement: bool`（含与 `requirement` 互斥校验、`SET requirement = NULL` 分支）。
- `worker.rs` `EditAnnotation`：空白提交 = 显式清空（内存 `None` + clear patch）；非空 = 原样落库；store 写失败 `tracing::warn!` 不再静默。
- 修复基线：6 个 TUI 测试文件 23 处陈旧调用点（函数签名新增菜单态参数后未同步），机械补参，未改断言。

## Validation

- `cargo test --workspace` → 2630 passed / 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- `cargo build --workspace` → Finished

### 测试覆盖

| 测试 | 断言 |
| --- | --- |
| `chat_req::tests::update_annotation_text_preserves_raw_bytes` | TAB/`\r`/BEL 原样保留 |
| `worker::tests::edit_annotation_sets_requirement` | 非空提交逐字节进 `sess.requirement`，循环不中断 |
| `worker::tests::edit_annotation_blank_clears_requirement` | 空白提交将 `Some("old")` 清为 `None` |
| `store::clear_requirement_nulls_requirement_field` | clear 置 NULL，无关列不受影响 |
| `store::default_patch_leaves_requirement_intact` | 普通 patch 不误清 |
| `store::clear_then_set_roundtrip` | 清空后可重新写入 |
| `store::field_and_clear_combinations_are_rejected`（新增 requirement 用例） | `requirement` + `clear_requirement` 互斥报错 |

## Compatibility

- `last_annotation_text` 的 first_prompt 回退语义保持（仅作编辑器种子，不再触发覆盖落库）。
- fork 不继承 annotation（`requirement: None`）为既有刻意行为，未改动。
- 原始文本含终端控制字符时渲染可能异常 —— 记录优先，仅渲染层可另行防护，存储串不改。
