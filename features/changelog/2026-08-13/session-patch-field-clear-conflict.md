Commit: (working-tree, pre-initial-commit)

# SessionPatch 字段+清除冲突前置拒绝 (Bug #15)

## 背景
`SessionPatch` 可同时「设置某列」与「清除同一列」（例如 `summary: Some(..)` +
`clear_summary: true`）。`libsql_store::sessions::update` 此前为这种矛盾输入
生成两条冲突的 `SET` 子句（如 `summary = ?` 与 `summary = NULL` 并存），最终生效
值取决于子句在生成 SQL 中的排列顺序 —— 不可预测且与列绑定语义相反。

## 变更
### 前置冲突校验
- **`crates/store/src/libsql_store/sessions.rs`**：
  - 新增纯函数 `validate_no_field_clear_conflict(&SessionPatch) -> Result<()>`，
    在 `update()` 入口最先调用。三族互斥规则：
    - `clear_summary` 与 `summary` / `summary_seq` / `summary_images` 互斥
      （`clear_summary` 一次 NULL 掉 `summary`+`summary_seq`+`summary_images_json`）；
    - `clear_handoff` 与 `handoff_seq` / `handoff_plan` 互斥；
    - `clear_skill` 与 `skill` 互斥。
  - 冲突时 `anyhow::bail!` 返回描述性错误，不再产生矛盾 SQL。
  - 错误返回沿用 store 既有 `anyhow::Result` + `bail!` 约定（见 events.rs/inputs.rs）。

### 调用方兼容性
审计全部生产调用方（`update_session` 的所有真实调用点），均无字段+清除冲突组合：
- `compaction.rs`：set summary 系 + `clear_handoff`（不同列族）；
- `control_cmd.rs::persist_clear`：set handoff/agent + `clear_summary`+`clear_skill`（不同列族）；
- `autopilot/phases.rs`：set handoff + `clear_summary`+`clear_skill`（不同列族）；
- `worker.rs`：set handoff + `clear_skill`（不同列）；
- `api_ops.rs`：`Some(skill)` 与 `clear_skill=true` 走互斥 match 臂，永不并存。
因此新校验对现有行为零影响。

## 测试覆盖
| 层级 | 测试名 | 文件 | 断言要点 |
|------|--------|------|----------|
| unit | `non_conflicting_patch_is_accepted` | `libsql_store/sessions.rs` | 合法组合（跨列、单独 clear、空 patch）均 `Ok` |
| unit | `summary_fields_with_clear_summary_are_rejected` | `libsql_store/sessions.rs` | summary/summary_seq/summary_images + clear_summary 均 `Err` |
| unit | `handoff_fields_with_clear_handoff_are_rejected` | `libsql_store/sessions.rs` | handoff_plan/handoff_seq + clear_handoff 均 `Err` |
| unit | `skill_with_clear_skill_is_rejected` | `libsql_store/sessions.rs` | skill + clear_skill `Err` |
| integration | `field_and_clear_combinations_are_rejected` | `tests/session_patch_conflict.rs` | 经 `update_session` 公开 API，6 个冲突对均 `Err` |
| integration | `unrelated_field_and_clear_still_succeeds` | `tests/session_patch_conflict.rs` | 跨列 set+clear 成功，且字段确实写入 |
| integration | `clear_flag_alone_succeeds` | `tests/session_patch_conflict.rs` | 单独 clear flag 成功 |

- 单元测试为零 I/O 纯函数测试（直接调 `validate_no_field_clear_conflict`），
  符合 rules/03 单元层；集成测试经真实 libsql `update_session` 路径，符合集成层。

## 回归门禁
- `cargo build --workspace` → 干净。
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- `cargo test --workspace` → **2436 passed; 0 failed; 0 ignored**（此前因
  `field_and_clear_combinations_are_rejected` 失败而红，现已转绿）。
- 防修绿扫描：0 删除 `#[test]`、0 新增 `#[ignore]`、0 弱断言、0 调试输出。
- 文件行数：`sessions.rs` 379 行（≤800）。
