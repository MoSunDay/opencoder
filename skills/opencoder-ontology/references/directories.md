# 实体目录与挂载

以下示例适用于鉴权已关闭的 Server；占位地址与 `<...>` ID 需替换为实际值。命令默认作用于 `debug` 空间，其他空间加 `--env <env-key>`；示例中的版本号替换为查询返回的当前 `expected_revision`。

实体目录通过目录归属关系组织父子目录和普通实体：关系从成员指向父目录，目录节点本身也以实体承载。目录负责层级归属，依赖等连接用普通关系表达，不要混用。

本页只维护目录结构、成员归属与解除；实体本身的创建与更新见 [records.md](records.md) 的「实体」。

## 目录树

```sh
opencoder-cli --no-auth ontology directories tree
opencoder-cli --no-auth ontology directories tree --include-deleted
opencoder-cli --no-auth ontology directories create --json '{"name":"组件目录"}'
opencoder-cli --no-auth ontology directories create --json '{"name":"子目录","parent_id":"<directory-id>"}'
opencoder-cli --no-auth ontology directories move <child-directory-id> --json '{"parent_id":"<new-parent-directory-id>"}'
opencoder-cli --no-auth ontology directories update <directory-id> --json '{"name":"更新目录名","is_deleted":false,"expected_revision":1}'
```

`tree` 返回整棵目录树和 `root_id`，是获取可用目录 ID 的首选入口；默认排除已删除目录，加 `--include-deleted` 读取保留数据。

目录树调整后重新执行 `tree` 读回确认；移动和改名不改变目录身份，成员归属自动跟随目录节点。

字段约定：

- 省略 `parent_id` 时创建在根目录下；根目录 ID 来自 `directories tree` 的 `root_id`。
- 子目录换父目录直接 `move`，不需要删除重建。
- 空目录通过 `update` 的 `is_deleted:true` 删除；非空目录会被服务端拒绝。
- 目录节点最多有一个父目录；`update` 仍需提供 `name`、`is_deleted`、`expected_revision`。

## 新建实体直接入目录

```sh
opencoder-cli --no-auth ontology entities create --json '{
  "request_id":"<request-uuid>","entity_type_id":"<type-id>",
  "name":"组件 B","source":"来源说明","ext":"补充说明",
  "directory_id":"<directory-id>"
}'
```

创建体带 `directory_id` 时直接建立归属关系；`request_id` 为 UUID，重试保持不变，`source`、`ext` 必填非空。

创建成功后用 `entities get <entity-id>` 确认目录归属已生效。目标目录先用 `directories tree` 确认存在且未删除；目录与实体必须属于同一数据空间。

## 已有实体挂载与解除

```sh
opencoder-cli --no-auth ontology relationship-types list
opencoder-cli --no-auth ontology relationships create --json '{
  "relationship_type_id":"<directory-membership-type-id>",
  "source_entity_id":"<entity-id>","target_entity_id":"<directory-id>"
}'
opencoder-cli --no-auth ontology relationships list --anchor <directory-id>
opencoder-cli --no-auth ontology relationships update <membership-relationship-id> \
  --json '{"description":"目录归属","is_deleted":true,"expected_revision":1}'
```

- 目录归属关系类型带有 `is_directory_membership:true` 标记，从 `relationship-types list` 动态发现并使用其 `id`，不要硬编码。
- 解除挂载即软删除对应归属关系。
- 普通实体可同时归属多个目录，多个归属可并存、互不影响；追加目录直接创建新的归属关系，无需先解除已有归属。
- 把实体从原目录移到新目录是两次独立请求：先解除原归属，再创建新归属；第二步失败可重新挂载。

挂载流程按顺序执行：

1. `relationship-types list` 找到目录归属关系类型的 `id`。
2. `relationships create` 建立从实体指向目录的归属。
3. `relationships list --anchor <directory-id>` 确认归属已存在。
4. 需要解除时 `relationships update` 软删除对应归属关系。

## 成员查询与方向

`relationships list --anchor <directory-id>` 返回的记录中，目录归属类型且 `target_entity_id` 为当前目录的行，其 `source_entity_id` 才是成员 ID，再用 `entities get` 读取详情。

目录自身的父级关系方向相反（目录作为 source 指向其父目录），不属于该目录成员。这些记录只表示直接成员，查询子目录成员需继续遍历目录树。

返回的关系记录包含 `relationship_type_id`、`source_entity_id`、`target_entity_id` 与 `expected_revision`；解除挂载时使用归属关系自身的 ID 与版本号，不是实体或目录的 ID。

查询步骤：

1. 从 `directories tree` 取目标目录 ID。
2. `relationships list --anchor <目录-id>` 拿到直接归属记录。
3. 按 `target_entity_id` 等于该目录过滤，逐个 `entities get <source-entity-id>` 读取成员详情。
4. 需要子目录成员时，对其子目录重复第 2、3 步。

## 服务端拒绝项

- 同一实体对同一目录的重复归属。
- 目录节点的第二个父目录与目录循环。
- 移动根目录；删除非空目录。

以上情况返回错误后按实际原因调整：先读目录树与成员清单确认现状，再调整请求内容（例如先移出成员再删除目录），不要绕过接口直接写库。命令成功后用 `directories tree` 或成员查询读回验证；只重试可确认结果的请求，不把局部成功写成全部成功。
