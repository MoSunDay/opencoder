# rdb 向量载荷加固（能力库 embedding）

一句话：能力库向量行（`vec/<id>`）的编解码与相似度收敛为严格契约：
`vector::cosine` 返回 `Option<f32>`，仅在维度完全一致且全部值有限时才有
定义；写路径在批次提交前拒绝非法载荷，读路径跳过损坏行而不是失败 ——
撕裂 blob、维度不符、非有限值永远不会被静默截断成一个「合法」低维向量
参与打分。属于 rdb 迁移的一部分（见
[rdb-store-and-event-framework.md](rdb-store-and-event-framework.md)）。

## 行为口径

- **cosine 语义**：维度必须完全一致（不静默截断到短侧），任一侧存在
  NaN/Inf 即无相似度（`None`）；有限输入仍可能把累加器溢出到非有限，
  非有限得分同样回答 `None`，零向量映射为 0。
- **写门（提交前）**：`dim <= 0`、blob 长度 `!= dim * 4`（过短、撕裂尾
  字节、过长）与任一解码值非有限都在 staging 之前拒绝 —— 没有批次可
  提交，已存向量不受影响；组合创建、版本守卫更新与裸 upsert 共用同一
  门（`validate_vector`）。
- **读路径（跳过而非失败）**：`search` 对 key 不可解析、元数据不可解码、
  model 不匹配、dim 非正、blob 与 dim 不符或含非有限的行直接跳过；`top_k`
  只对有定义的相似度排序 —— 损坏行不会挤掉合法命中，维度与存库向量不符
  的查询得空结果而不是截断打分。

## 变更清单

- `crates/store/src/rdb_store/vector.rs` — LE f32 编解码
  （`decode_f32_le_exact`：恰好 `dim * 4` 字节且全有限）、`cosine`、
  `top_k`（跳过不可比较候选，并列按输入顺序稳定）。
- `crates/store/src/rdb_store/execution/brain.rs` — `validate_vector`
  写门、`searchable_row` 跳过逻辑与 brute-force `search`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 写门拒绝（dim/长度/非有限）且零提交，三写路径共用 | `invalid_vector_payloads_are_rejected_and_commit_nothing` | `crates/store/tests/rdb_vectors/suite_vector_validation.rs` |
| 损坏行（坏元数据/维度不符/非有限）跳过，合法命中保留 | `search_skips_corrupt_vector_rows_instead_of_failing` | 同上 |
| 组合创建/更新原子落库并立即可搜 | `create_with_vector_persists_capability_inputs_and_vector_together`、`update_with_vector_replaces_content_inputs_and_vector` | `crates/store/tests/rdb_vectors/suite_vector.rs` |
| upsert 幂等与能力外键 | `upsert_vector_is_idempotent_and_requires_the_capability` | 同上 |
| 余弦距离排序与 limit | `vector_distance_orders_hits_and_reports_cosine_distance` | 同上 |
| embedding model 作用域 | `search_filters_by_embedding_model` | 同上 |

- 定向回归（只覆盖本链路，非全量）：

  ```
  cargo test -p opencoder-store --test rdb_vectors --quiet
  ```
