# 项目执行参考补齐与 Ontology Skill

## 行为

- 新增可安装 Skill `skills/opencoder-ontology/`：SKILL.md 只保留路由与核心约定（连接复用根 CLI 鉴权开关、从能力目录起步、`--env` 默认 debug、`expected_revision` 乐观锁、`update` 软删除、实体创建 UUID `request_id` 与 `source/ext` 必填、平台生成字段 `management.managed_by:"platform-catalog"` 不可直接改、`graph` 不接收切面 ID、退出码 0/1/2/4 且 `sync check` 差异返回 4），命令细节拆入四个参考文档：capabilities-sync（能力目录与同步）、schema（数据空间与类型定义）、records（实体、关系、图谱、切面与向量）、directories（目录树、挂载与成员查询），另附 `agents/openai.yaml` 接口描述。目录语义与当前规则一致：目录节点单父、普通实体可多目录归属，同一实体对同一目录的重复归属被拒。
- 补齐 `skills/opencoder-projects/`：原「CRUD 不发起执行时查 --help」的指引改为指向新增 `references/execution.md`；看板排序入口写入 SKILL.md；`references/crud.md` 增加 `todos reorder`（`--json` 传入 `initiative_id/board_status/ids`）与 `tags list --scope-type/--scope-id` 示例。`references/execution.md` 覆盖执行关联（links/attach/detach/dispatch 含稳定执行 ID 与重试冻结语义）、运行编排（plan/execute 202 与 `prun-` 前缀 `run_id`、runs 的 `--before-version` 游标、cancel-run 幂等）与会话操作（prompt 的 `delivery steer|queue`、`prompt/interrupt` 仅限 agent/operator 关联、messages 游标与 events `--after`）。
- [Ontology 功能说明](../../ontology/index.md) 的 CLI 段落补充 opencoder-ontology skill 链接，对齐项目功能说明引用 projects skill 的写法。示例均为占位地址与占位 ID，不含凭证。

## 测试覆盖

| 范围 | 测试名 / 入口 | 文件 |
| --- | --- | --- |
| Ontology 命令映射与 CLI 契约 | `ontology_commands_map_to_native_methods_paths_and_revisions` 等 3 项 | `crates/ctl/tests/parse_ontology.rs` |
| 项目 CRUD、运行编排与会话命令解析 | `project_details_tags_order_and_conversation_commands_parse` 等 13 项 | `crates/ctl/tests/parse_project_brain_agents.rs` |
| Skill frontmatter、相对链接与行数 | 名称匹配 `[a-z][a-z0-9-]*` 且等于目录名；新增文件 ≤400 行 | `skills/opencoder-ontology/`、`skills/opencoder-projects/` |

两份 Skill 为纯文档改动：示例命令逐条对照 CLI 命令定义与解析测试交叉核对，未改任何代码；文档改动仅检查内容与链接。两份 Skill 的端到端验证通过（2026-10-30 用户确认）；未执行全量测试。

相关：[Ontology 功能说明](../../ontology/index.md) · [项目功能说明](../../project/index.md) · [Ontology CLI](../../../rules/10-ontology-cli.md) · [opencoder-ontology skill](../../../skills/opencoder-ontology/SKILL.md) · [opencoder-projects skill](../../../skills/opencoder-projects/SKILL.md)
