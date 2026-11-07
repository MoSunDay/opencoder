# legacy 迁移支持更老的 schema 世代（缺列补 NULL、缺表计零行）

一句话：真实机器上的 legacy `opencoder.db` 跨越多个 schema 世代（后期才
追加的列如 `kind`/`requirement`/`provider_state_json`/`display`/
`sse_kind`，以及后期才出现的 `subagent_tasks` 表），原迁移读取器按最终
schema 硬 SELECT，遇老库直接 `no such column` 失败。现在按
`pragma_table_info` 对每张表做动态投影：存在的列按名读取，缺失的列以
`NULL AS <col>` 补位（后期列均可为空），缺失的表贡献零行。

## 变更清单

- `crates/local/src/legacy/extract.rs` — SELECT 常量改为 `TableSpec`
  （表名、列序、过滤、排序）；新增 `table_rows`/`column_set`/`query_rows`：
  缺表跳过、缺列投影为 `NULL`，结果布局恒等于解析器钉死的列序。
- `crates/local/src/legacy/parse.rs` — 移除 `require_columns`（投影后
  列序由构造保证，值仍严格解码，绝不静默矫正）。

## 行为口径

- 列容忍仅限"后期追加的可空列"：值级严格解码不变（未知枚举、坏 JSON、
  浮点时间戳等仍是硬错误）。
- 缺表（如老库无 `subagent_tasks`）计零行并照常写 marker/backup；
  `sessions` 表缺失仍判定为非会话存储而拒绝。
- 实测采样（本机 5121 个 legacy 库）：仅 113 个是最终 schema，5004 个
  老世代库此前全部迁移失败；本修复后均可按上述口径迁移。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 老世代 schema 端到端迁移（缺列 + 缺表 + 物理列序不同） | `migrates_older_schema_generation_with_null_projection` | `crates/local/tests/legacy_schema_variants.rs` |
| 最终 schema 全量语义回归 | `legacy_migration` 套件 12 例 | `crates/local/tests/legacy_migration.rs` |

- 定向回归：`cargo test -p opencoder-local --lib`（106）、
  `--test legacy_migration`（12）、`--test legacy_schema_variants`（1）
  全部通过；`cargo clippy -p opencoder-local --tests` 零警告。
- 测试范围：仅 legacy 迁移读取链路；未运行全量测试。
