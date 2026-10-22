Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# 规则 10：Ontology CLI 操作约定

Ontology 的定位、对象组织和变更同步要求见 [规则 09](09-ontology-capabilities.md)。本文维护 `opencoder-cli ontology` 的命令入口和实际操作方式；CLI 或 API 约定变化时同步更新本文。

## 连接与写入约定

CLI 使用已有 Server 连接配置和鉴权。默认数据空间为 `debug`，通过 `opencoder-cli ontology --env <env-key> ...` 指定其他空间。服务端自动同步当前 Server 的 `debug`；其他空间由调用方显式同步。

- 写入体使用 `--json '<JSON>'` 或 `--json @file.json`，字段与原 API 相同。
- 更新携带查询返回的当前 `expected_revision`；发生版本冲突时重新读取，不覆盖他人修改。
- 新建实体携带 UUID 格式的 `request_id`，同一次创建重试保持 ID 和内容不变；`source`、`ext` 必填。
- 除切面有独立 `delete` 命令外，实体、关系、类型、属性、Action、目录和环境均通过 `update` 设置 `is_deleted:true` 软删除，仍需提供对应更新体的必填字段。
- 普通列表排除已删除对象；`--include-deleted` 可读取保留的数据。Action 命令维护操作配置，本身不执行 Action。
- 平台生成字段通过原模块修改；补充说明、自定义属性及自定义关系可以维护，后续同步保留。

成功响应写入 stdout，失败写入 stderr。退出码：0 成功，1 传输失败，2 鉴权或权限失败，4 服务端拒绝或同步检查发现差异。具体命令参数可用 `--help` 查询。

## 从能力目录开始

```sh
opencoder-cli ontology session
opencoder-cli ontology capabilities list
opencoder-cli ontology capabilities list --kind dag --query example
opencoder-cli ontology capabilities get <entity-id>
opencoder-cli ontology sync status
opencoder-cli ontology sync check
opencoder-cli ontology sync run
```

能力列表返回 `items` 和筛选后的 `total`，支持 `--offset`、`--limit`、`--include-deleted`。每项包含稳定的 Ontology 实体 ID、`capability` 与停用标记。能力描述包含来源标识、类型、版本、输入输出要求、CLI/API 入口及明确依赖。

`sync check` 只读比较来源与已同步内容，包括平台生成字段是否偏离；一致返回 0，存在差异返回 4。`sync run` 等待一次完整同步，返回新增、更新、停用及修复数量。同步后再次 `sync check` 确认一致。

同步状态包含 `not_initialized`、`running`、`ready`、`error`、最后成功时间和错误信息；`issues` 列出无法解析的引用。来源读取或写入失败时保留最后成功内容，后台持续重试。自动同步可能有短暂延迟，不改变原模块的保存回执。

## 命令清单

| 命令组 | 操作 |
| --- | --- |
| `session` | 查询当前身份和权限 |
| `capabilities` | `list`、`get` |
| `sync` | `status`、`check`、`run` |
| `environments`、`entity-types`、`relationship-types` | `list`、`create`、`update` |
| `attributes`、`actions` | 按实体类型 `list`、`create`，按属性或 Action ID `update` |
| `entities` | `list`、`get`、`create`、`update`、`set-value`、`set-text`、`set-nfs-path`、`text-history`、`text-get` |
| `relationships` | `list`（可加 `--anchor`）、`create`、`update` |
| `directories` | `tree`、`create`、`update`、`move` |
| `aspects` | `list`、`get`、`create`、`update`、`delete --expected-revision` |
| `graph` | 多个 `--center`、类型和关系筛选、上下游深度、`--pinned-only`、`--expand-neighbors` |
| `vectors` | `get`、`set`、`search` |
| `nfs` | `status`、`set`；管理正文只读导出 |

## 实体、关系与切面

以下 `<...>` 均替换为实际 ID，版本号示例替换为查询返回的当前版本。

```sh
opencoder-cli ontology entity-types create --json '{"key":"component","name":"组件"}'
opencoder-cli ontology entity-types list
opencoder-cli ontology entities create --json '{
  "request_id":"<request-uuid>","entity_type_id":"<type-id>",
  "name":"组件 A","source":"来源说明","ext":"补充说明"
}'
opencoder-cli ontology entities list --offset 0 --limit 100
opencoder-cli ontology entities get <entity-id>
opencoder-cli ontology entities update <entity-id> --json '{
  "name":"组件 A（更新）","description":"说明","is_deleted":false,"expected_revision":1
}'
opencoder-cli ontology relationship-types create --json '{
  "key":"depends","name":"依赖","source_entity_type_id":"<type-id>",
  "target_entity_type_ids":["<type-id>"]
}'
opencoder-cli ontology relationships create --json '{
  "relationship_type_id":"<relationship-type-id>",
  "source_entity_id":"<source-id>","target_entity_id":"<target-id>"
}'
opencoder-cli ontology relationships list --anchor <entity-id>
opencoder-cli ontology graph --center <entity-id> \
  --entity-type-id <type-id> --relationship-type-id <relationship-type-id> \
  --upstream-depth 1 --downstream-depth 2
opencoder-cli ontology aspects create --json '{
  "aspect_key":"component-dependencies","name":"组件依赖",
  "entity_type_ids":["<type-id>"],"relationship_type_ids":["<relationship-type-id>"],
  "default_center_ids":["<entity-id>"],
  "default_upstream_depth":1,"default_downstream_depth":2
}'
opencoder-cli ontology aspects list
opencoder-cli ontology aspects get <aspect-id>
opencoder-cli ontology aspects update <aspect-id> --json '{
  "name":"组件依赖（更新）","entity_type_ids":["<type-id>"],
  "relationship_type_ids":["<relationship-type-id>"],
  "default_center_ids":["<entity-id>"],
  "default_upstream_depth":1,"default_downstream_depth":3,
  "is_deleted":false,"expected_revision":1
}'
```

`relationships list --anchor` 返回该实体作为源或目标的关系；不传 `--anchor` 则返回全部关系。切面保存的条件可通过 `aspects get` 读取，再传给 `graph` 进行查询；当前 `graph` 没有直接接收切面 ID 的参数。

软删除示例：

```sh
opencoder-cli ontology entities update <entity-id> --json '{
  "name":"组件 A（更新）","description":"说明","is_deleted":true,"expected_revision":2
}'
opencoder-cli ontology relationships update <relationship-id> --json '{
  "description":"关系说明","is_deleted":true,"expected_revision":1
}'
opencoder-cli ontology aspects delete <aspect-id> --expected-revision 2
opencoder-cli ontology entities list --include-deleted
opencoder-cli ontology relationships list --include-deleted
opencoder-cli ontology aspects list --include-deleted
```

## 实体目录与挂载

实体目录通过目录归属关系组织父子目录和普通实体。

```sh
opencoder-cli ontology directories tree
opencoder-cli ontology directories create --json '{"name":"组件目录"}'
opencoder-cli ontology directories create --json '{
  "name":"子目录","parent_id":"<directory-id>"
}'
opencoder-cli ontology directories move <child-directory-id> \
  --json '{"parent_id":"<new-parent-directory-id>"}'
opencoder-cli ontology directories update <directory-id> --json '{
  "name":"更新目录名","is_deleted":false,"expected_revision":1
}'
```

省略 `parent_id` 时创建在根目录下，根目录 ID 来自 `directories tree` 的 `root_id`。新建实体时增加 `directory_id`，创建成功后即归入该目录：

```sh
opencoder-cli ontology entities create --json '{
  "request_id":"<request-uuid>","entity_type_id":"<type-id>",
  "name":"组件 B","source":"来源说明","ext":"补充说明",
  "directory_id":"<directory-id>"
}'
```

已有实体通过内置目录归属关系挂载。先从 `relationship-types list` 找到 `is_directory_membership:true` 的关系类型，再使用其 `id`：

```sh
opencoder-cli ontology relationship-types list
opencoder-cli ontology relationships create --json '{
  "relationship_type_id":"<directory-membership-type-id>",
  "source_entity_id":"<entity-id>","target_entity_id":"<directory-id>"
}'
opencoder-cli ontology relationships list --anchor <directory-id>
opencoder-cli ontology relationships list --anchor <entity-id>
```

查询目录成员时，在返回关系中筛选该目录归属类型且 `target_entity_id` 为当前目录的记录，`source_entity_id` 就是成员 ID；用 `entities get` 继续读取详情。目录自身的父级关系方向相反，不属于该目录的成员。这些记录表示直接成员，查询子目录的成员需继续遍历目录树。

一个实体最多有一个父目录。解除挂载时，将对应目录归属关系软删除：

```sh
opencoder-cli ontology relationships update <membership-relationship-id> \
  --json '{"description":"目录归属","is_deleted":true,"expected_revision":1}'
```

普通实体换目录时，先解除原归属，再创建新归属。这是两次独立请求，第二步失败时可重新挂载。子目录换父目录直接使用 `directories move`。服务端拒绝重复归属、目录循环、移动根目录及删除非空目录；空目录通过 `directories update` 的 `is_deleted:true` 删除。

## API 与验证入口

能力查询和同步 API 位于 `/api/ontology/envs/:env`：

- `GET /capabilities`、`GET /capabilities/:id` 查询能力。
- `GET /sync` 查询状态，`GET /sync/check` 只读比较。
- `POST /sync` 执行并等待一次同步。

读取沿用 Ontology 读权限，手动同步沿用写权限。已有实体、图谱和正文 API 可访问同步生成的数据。

CLI 实现见 [命令模块](../crates/ctl/src/cmd/ontology/mod.rs)。命令映射见 [解析测试](../crates/ctl/tests/parse_ontology.rs)，真实 CLI 与服务端操作见 [集成测试](../crates/ctl/tests/ontology_server.rs)及其 [目录](../crates/ctl/tests/ontology_server/directories.rs)、[实体、关系与切面](../crates/ctl/tests/ontology_server/records.rs)、[类型与环境](../crates/ctl/tests/ontology_server/schema.rs)用例。按本次改动范围选择测试，不默认运行全量测试。
