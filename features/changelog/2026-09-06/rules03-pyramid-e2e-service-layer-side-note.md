# rules/03 金字塔图 e2e 顶层补「按真服务判层」侧注

日期：2026-09-06

## 需求与实现

- 上轮评审（`1a5104a` 判层标准修正轮）遗留 TODO #1（低，可选装饰）：金字塔图顶层 `/e2e\` 标签仍可能被误读为「路径=层」。本轮在图示顶层注记 `scripts/e2e-glm.sh` 旁追加侧注「（按真服务判层）」，与 `rules/03` 第 3 层判层标准条目（是否依赖真外部服务、不以路径名判层）显式互指，路径误读残留清零。
- 纯文档单行变更，零 `.rs` 触碰；ASCII 图对齐不变（侧注追加于注记列右侧）。

## 测试清单（当次实跑）

- `cargo test -p opencoder-control --test e2e` → 155 passed / 0 failed / 0 filtered out（exit 0）
- `cargo fmt --all --check` → exit 0
- workspace/clippy 免跑链延续：`b70be00..HEAD` 全 docs-only + 零脏 `.rs`；首个 `.rs` 变更落地即触发 rules/02 全量 gate 重跑。

## 部署备注

无生产行为变化。
