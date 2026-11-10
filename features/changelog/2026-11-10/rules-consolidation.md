# 仓库规则精简：11 条合并为 9 条

## 概述

- [rules/](../../../rules) 由 11 条精简为 9 条（总行数 951 → 743），全部规范性内容（阈值、禁令、路径、入口）保留，仅去重与压缩；[rules/README.md](../../../rules/README.md) 索引与快速检查清单同步重写。

## 要点

- 规则 11（测试组织）并入 [规则 03](../../../rules/03-test-pyramid.md)，标题改为「测试分层与组织」：分层判层、取证规范在前，目录落位、夹具复用与验收脚本归组在后；模块地图去掉文件计数，只锁目录级防漂移。
- 规则 10（Ontology CLI）并入 [规则 09](../../../rules/09-ontology-capabilities.md)，标题改为「Ontology 能力目录与 CLI」：实体目录挂载、软删除与同步检查约定合并为一处，CLI 操作约定紧随能力变更流程。
- 规则 01/02/04-08 措辞压缩：06 删除 mermaid 调度示意图（文字条款与门槛不变），07 拆分超长布局句，其余去冗余；语义不变。
- 引用同步：AGENTS.md、repo-memory.md、rules/README.md、agents/platform/ctl、features/platform/ontology 改指合并后的文件；本次补齐 repo-memory.md 仓库规则清单中遗留的规则 11 条目（并入 03 的描述）。

## 测试覆盖

| 检查 | 结果 |
|------|------|
| `grep -rn "10-ontology-cli\|11-test-organization" rules/ AGENTS.md repo-memory.md agents/ features/`（排除 changelog 与 skills 快照） | 修复 repo-memory.md 遗留条目后 0 残留 |
| rules/ 内部相对链接抽查（README → 01-09、03 → repo-memory、06/07 → crates 与 features、09 → ctl/features） | 目标均存在，0 条悬空 |
| `wc -l rules/*.md` | 全部 ≤ 400 行（最大 09 = 183），总计 951 → 743 |

- 测试范围：纯文档改动，按规则 02 只做内容与链接检查，未运行代码测试。

## 相关链接

- [rules/README.md](../../../rules/README.md)
- [rules/03-test-pyramid.md](../../../rules/03-test-pyramid.md)
- [rules/09-ontology-capabilities.md](../../../rules/09-ontology-capabilities.md)
