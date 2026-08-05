# handoff 持久化未清除残留 compaction 元数据 → transcript 污染

## 背景

`compaction.rs` 计算 skip 偏移为 `prev_skip = summary_seq.or(handoff_seq)`。compaction 持久化路径设了 `clear_handoff: true`（清除 handoff），但 handoff 持久化路径**没有对称的 `clear_summary`**。当 session 经历 compaction（`summary_seq=10`）→ handoff（`handoff_seq=28`）→ resume → 再次 compaction 时，残留的较小 `summary_seq=10` 优先于 `handoff_seq=28`，OFFSET 偏小，已 summarize 的旧消息被重新加载，污染上下文。

## 变更

### 写入层（主防线）

- **`store/src/types.rs`**：`SessionPatch` 新增 `clear_summary: bool`（对称于 `clear_handoff`，None=skip 语义无法表达「置 NULL」）。
- **`store/src/libsql_store/sessions.rs`**：`update()` 加 `clear_summary` 分支 → `summary`/`summary_seq`/`summary_images_json = NULL`。
- **同上**：`create()` INSERT 补 `summary_images_json` 列（修复 INSERT 与 struct 读写不对称：新 session 的 image 列此前被静默丢弃）。
- **`session/src/control_cmd.rs`**：`persist_clear()` 设 `clear_summary: true`（/clear 上下文切换）。
- **`session/src/autopilot/phases.rs`**：autopilot handoff 设 `clear_summary: true`。

### resume 层（防御性，覆盖已落盘脏数据）

- **`session/src/resume.rs`**：handoff 分支 `meta.handoff_seq.is_some()` → summary/summary_seq/summary_images 置空，确保 `prev_skip` 取 `handoff_seq` 而非残留的较小 `summary_seq`。对修复前创建的脏 session 仍生效。

## 测试覆盖

| 断言 | 测试名 | 文件 |
|------|--------|------|
| clear_summary NULL 三个 compaction 列 | `clear_summary_nulls_all_compaction_fields` | `store/tests/clear_summary.rs` |
| clear_summary 与 handoff 更新在同一次 patch 共存不互踩 | `clear_summary_coexists_with_handoff_update` | `store/tests/clear_summary.rs` |
| create() INSERT 绑定 summary_images_json（round-trip） | `create_persists_summary_images` | `store/tests/clear_summary.rs` |
| resume 后 SessionState.summary_seq 为 None（脏数据被清） | `resume_handoff_clears_stale_summary_seq` | `session/tests/handoff_clears_compaction.rs` |
| 修复后 compaction OFFSET 用 handoff_seq（new_skip=8 非 6） | `clear_summary_prevents_offset_corruption` | `session/tests/handoff_clears_compaction.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 1889 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |

行数约束：新增文件 `clear_summary.rs`(126) / `handoff_clears_compaction.rs`(253) 均 ≤400 行。
