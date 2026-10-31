# 仓库逻辑地图刷新：agents/features 索引与 repo-memory

一句话：40 个模块/能力索引与 repo-memory.md 按当前代码刷新——Commit 锚点统一到 a8a742ba7、清理已删文件引用（如 DirectoryActions.tsx）与过期表述（brain schema 7 里程碑、server 控制面旧定位）、补上新规则（05 UI 验收 / 09 Ontology 能力 / 10 Ontology CLI）与 Responses 能力入口链接。

## 变更清单

- `repo-memory.md`：定位行补 `mcp_mock_server` 测试二进制；server 定位改为「版本与只读资源服务」；规则索引补 05/09/10；brain 摘要去掉过期 schema 7 表述。
- `agents/*/index.md`（24 个）与 `agents/web/ontology.md`、`agents/worker/journal.md`：锚点更新、过期文件/模块引用清理、现行结构补齐。
- `features/*/index.md`（13 个）：锚点更新、能力入口补齐（`features/index.md` 增加 Responses API 链接）、过期描述收缩。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 索引内相对链接有效性 | 内联链接检查：全部变更 md 的相对路径逐一解析，无 BROKEN | — |

## 定向回归

- 变更 md 相对链接检查 → 无失效链接。
- 纯文档变更，无代码/测试影响面（规则 02：文档只查内容与链接）。

## 配置与依赖

- 无。

## 回滚步骤

- 还原对应文档文件即回滚；无数据迁移。

## 遗留

- 索引 `Commit:` 锚点为信息性字段、无脚本校验，随下次代码变更再同步。
