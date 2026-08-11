# 文档测试计数去硬编码 + 冗余 dead_code/桩函数清理

## Context

代码库编译零错误、clippy 零警告、测试全绿。本轮聚焦两类卫生问题：
文档中硬编码的测试计数随迭代漂移（写死的数字很快过期），以及代码实际在用却仍挂着
`#[allow(dead_code)]` 的冗余标注与仅为压制 unused-import 警告而存在的空桩函数。

## Change Summary

**文档去硬编码**（避免数字再次漂移）

- `agents/session/index.md`、`features/index.md`：「测试规则与覆盖」段移除硬编码的
  「workspace 测试 1406 个」，改为指向 `cargo test` 当前输出，不再随迭代失真。
- `features/changelog/2026-08-11/tui-attachment-cursor-alignment.md`：Validation 段
  过期计数 `2340` 校正为 `2345`（与该次提交时的回归结果对齐）。

**删除冗余 `#[allow(dead_code)]`**（标的均经确认在用，删除后 clippy 无新警告）

- `crates/web/src/handle.rs`：`DropGuardStream` 结构体、`DropGuardStream::new`、
  `release_events_subscriber` 三处。
- `crates/store/src/libsql_store/sessions.rs`：`extract_preview`。

**删除无用桩函数 + 随之失效的 import**

- `crates/session/src/resume.rs`：删除 `_ensure_agent_used`，并移除仅被它引用的 `Agent`
  import。
- `crates/session/tests/output_streamline.rs`：删除 `_types`，并移除仅被它引用的 `Message`
  import。

全部改动为属性/死代码/导入/文档的纯清理，无 trait、store 数据形状、CLI、HTTP、
prompt 契约变化，无行为变更。

## Validation

- `cargo build --workspace` → Finished，零错误零警告。
- `cargo clippy --workspace --all-targets -- -D warnings` → Finished，零警告
  （证明删除 allow 后无任何 dead_code 暴露、删除桩后无 unused-import）。
- `cargo test --workspace` → `total passed=2347 failed=0`（全二进制汇总，0 failed）。

## 测试覆盖

本轮为文档/死代码清理，无新增业务功能，故无新增测试（N/A）。改动均属非行为性移除，
其正确性由 `clippy --all-targets -D warnings` 零警告（含被改测试文件 output_streamline.rs
在内的全部 target 编译干净）+ 既有测试不回归共同保证。

## Related Docs

- [agents/session](../../agents/session/index.md)
