# 实体、关系、图谱、切面与向量

以下示例适用于鉴权已关闭的 Server；占位地址与 `<...>` ID 需替换为实际值。命令默认作用于 `debug` 空间，其他空间加 `--env <env-key>`；引用对象必须属于同一空间。示例中的版本号替换为查询返回的当前 `expected_revision`。

## 实体

```sh
opencoder-cli --no-auth ontology entities list --offset 0 --limit 100
opencoder-cli --no-auth ontology entities list --include-deleted
opencoder-cli --no-auth ontology entities get <entity-id>
opencoder-cli --no-auth ontology entities create --json '{
  "request_id":"<request-uuid>","entity_type_id":"<type-id>",
  "name":"组件 A","description":"示例实体","source":"来源说明","ext":"补充说明"
}'
opencoder-cli --no-auth ontology entities update <entity-id> --json '{
  "name":"组件 A（更新）","description":"说明","is_deleted":false,"expected_revision":1
}'
```

字段约定：

- 创建体的 `request_id` 必须是 UUID，同一次创建重试保持 ID 和内容不变。
- `source`、`ext` 必填非空：`source` 记录来源，`ext` 记录补充说明。
- 可选 `directory_id` 把新实体直接归入目录，见 [directories.md](directories.md)。
- 可选 `attributes` 按属性定义 ID 传入初始值。
- 更新体：`{"name","description"?,"is_deleted","expected_revision"}`；软删除把 `is_deleted` 设为 `true`。

实体列表返回分页数据；`get` 返回实体、属性值与关联关系的完整详情。平台生成的实体带 `management` 信息，`managed_by:"platform-catalog"` 与 `readonly_attributes` 的约定见 SKILL.md 的核心约定。

创建超时先按相同 `request_id` 幂等重试，再按名称、类型和时间在列表中查重，不盲目重复创建。命令成功后用 `entities get` 读回验证名称、类型与目录归属。

## 属性值与正文

```sh
opencoder-cli --no-auth ontology entities set-value <entity-id> <attribute-id> --json '{"kind":"text","value":"负责人示例","is_deleted":false,"expected_revision":1}'
opencoder-cli --no-auth ontology entities set-text <entity-id> <attribute-id> --json '{"format":"md","content":"# 说明\n正文","expected_revision":1}'
opencoder-cli --no-auth ontology entities set-nfs-path <entity-id> <attribute-id> --json '{"path":"/exports/ontology/example.md","expected_revision":1}'
opencoder-cli --no-auth ontology entities text-history <entity-id> <attribute-id>
opencoder-cli --no-auth ontology entities text-get <entity-id> <attribute-id> 2
```

- `set-value` 的 `kind` 必须与属性定义一致；必填属性不能清空或软删除值。
- `set-text` 用于 markdown 属性，要求 `format:"md"`，保存为新版本；历史版本保留。
- `set-nfs-path` 只用于补充说明角色且 NFS 存储的属性，路径指向正文文件。
- `text-history` 列出版本，`text-get` 按版本号读取正文，版本号是必填的位置参数。

属性值写入同样携带该值当前的 `expected_revision`。结构化属性用 `set-value` 一次写入；长正文先在临时文件编辑，再以 `--json @/absolute/path.json` 提交，避免 shell 转义出错。

## 关系

```sh
opencoder-cli --no-auth ontology relationships list
opencoder-cli --no-auth ontology relationships list --anchor <entity-id>
opencoder-cli --no-auth ontology relationships list --include-deleted
opencoder-cli --no-auth ontology relationships create --json '{
  "relationship_type_id":"<relationship-type-id>",
  "source_entity_id":"<source-entity-id>","target_entity_id":"<target-entity-id>",
  "description":"依赖说明"
}'
opencoder-cli --no-auth ontology relationships update <relationship-id> --json '{"description":"依赖说明（更新）","is_deleted":false,"is_pinned":null,"expected_revision":1}'
```

字段约定：

- 创建体：`{"relationship_type_id","source_entity_id","target_entity_id","description"?}`。
- 两端类型必须匹配关系类型定义且属于同一空间。
- 更新体：`{"description"?,"is_deleted","is_pinned"?,"expected_revision"}`；`is_pinned` 控制是否置顶展示。
- 关系没有独立 delete 命令，软删除走 `update` 的 `is_deleted:true`。

`--anchor` 返回该实体作为源或目标的关系；不传则返回全部关系。关系必须有明确来源，不通过名称或自然语言猜测依赖。置顶展示的关系在 `update` 中用 `is_pinned:true` 标记，`null` 表示不变更。

## 图谱与切面

```sh
opencoder-cli --no-auth ontology graph --center <entity-id> --depth 2
opencoder-cli --no-auth ontology graph --center <entity-id> \
  --entity-type-id <type-id> --relationship-type-id <relationship-type-id> \
  --upstream-depth 1 --downstream-depth 2
opencoder-cli --no-auth ontology graph --center <entity-id> --pinned-only --expand-neighbors
opencoder-cli --no-auth ontology aspects list
opencoder-cli --no-auth ontology aspects get <aspect-id>
opencoder-cli --no-auth ontology aspects create --json '{
  "aspect_key":"component-dependencies","name":"组件依赖",
  "entity_type_ids":["<type-id>"],"relationship_type_ids":["<relationship-type-id>"],
  "default_center_ids":["<entity-id>"],
  "default_upstream_depth":1,"default_downstream_depth":2
}'
opencoder-cli --no-auth ontology aspects update <aspect-id> --json '{
  "name":"组件依赖（更新）","entity_type_ids":["<type-id>"],
  "relationship_type_ids":["<relationship-type-id>"],
  "default_center_ids":["<entity-id>"],
  "default_upstream_depth":1,"default_downstream_depth":3,
  "is_deleted":false,"expected_revision":1
}'
opencoder-cli --no-auth ontology aspects delete <aspect-id> --expected-revision 2
```

- `graph` 的 `--center`、`--entity-type-id`、`--relationship-type-id` 可重复传值。
- 深度用统一的 `--depth` 或分列的 `--upstream-depth/--downstream-depth`。
- `--pinned-only` 只看置顶关系，`--expand-neighbors` 展开邻接节点。
- 切面保存的观测条件用 `aspects get` 读取后转成 graph 参数；`graph` 不直接接收切面 ID。
- 切面是唯一有独立 `delete` 命令的对象，需携带 `--expected-revision`。

图谱用于回答“这个能力依赖谁、被谁依赖”；范围过大时先收窄类型与关系筛选，再逐步放宽深度。反复使用同一组条件时保存为切面，下次直接读取条件复用，不重复手写参数。

## 向量与 NFS 正文导出

```sh
opencoder-cli --no-auth ontology vectors get <vector-id>
opencoder-cli --no-auth ontology vectors set <entity-id> <attribute-id> --json '{"vector":[0.1,0.2],"expected_revision":1}'
opencoder-cli --no-auth ontology vectors search --json '{"attribute_definition_id":<attribute-id>,"vector":[0.1,0.2],"limit":20}'
opencoder-cli --no-auth ontology nfs status
opencoder-cli --no-auth ontology nfs set --json '{"enabled":true}'
```

- 向量属性先在类型上定义为 `vector` kind，见 [schema.md](schema.md)。
- `set` 可选 `vector_id` 覆盖指定向量，`is_deleted` 软删除。
- `search` 按属性定义 ID 与查询向量返回邻近实体，`limit` 上限 100。
- `nfs status/set` 管理正文的只读 NFS 导出，`enabled:false` 停止导出；导出内容只读，写入仍走 CLI。

向量写入与查询必须在同一属性定义内比较；跨属性定义的向量 ID 不能混用。`search` 返回的是邻近实体列表，结果解释回到实体详情，不直接当作执行入口。

## 软删除示例

```sh
opencoder-cli --no-auth ontology entities update <entity-id> --json '{
  "name":"组件 A（弃用）","description":"说明","is_deleted":true,"expected_revision":2
}'
opencoder-cli --no-auth ontology relationships update <relationship-id> --json '{
  "description":"依赖说明","is_deleted":true,"expected_revision":1
}'
opencoder-cli --no-auth ontology aspects delete <aspect-id> --expected-revision 2
opencoder-cli --no-auth ontology entities list --include-deleted
opencoder-cli --no-auth ontology relationships list --include-deleted
opencoder-cli --no-auth ontology aspects list --include-deleted
```

软删除仍需提供对应更新体的必填字段；切面走独立 `delete` 并携带 `--expected-revision`。删除后用 `--include-deleted` 读回确认数据仍在列表中保留。

## 通用约定

- 退出码：0 成功；1 传输失败；2 鉴权或权限失败；4 服务端拒绝。
- 更新前先读取当前 `expected_revision`，冲突时重读，不覆盖他人修改。
- 列表默认排除已删除对象；`--include-deleted` 读取保留数据。
- 命令成功后读取对象验证关键字段；具体参数以 `--help` 为准。
