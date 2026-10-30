# 数据空间与类型定义

以下示例适用于鉴权已关闭的 Server；占位地址与 `<...>` ID 需替换为实际值。命令默认作用于 `debug` 空间，其他空间加 `--env <env-key>`；引用对象必须属于同一空间。写入体使用 `--json '<JSON>'` 或 `--json @file.json`，字段与原 API 相同。

## 数据空间

数据空间隔离一组类型、实体、关系和切面：

```sh
opencoder-cli --no-auth ontology environments list
opencoder-cli --no-auth ontology environments list --include-deleted
opencoder-cli --no-auth ontology environments create --json '{"key":"staging","name":"预发空间"}'
opencoder-cli --no-auth ontology environments update <env-id> --json '{"name":"预发空间（更新）","is_deleted":false,"expected_revision":1}'
```

- 创建体：`{"key","name","description"?}`。
- 更新体：`{"name","description"?,"is_deleted","expected_revision"}`。
- 两者均必填 `name`、`is_deleted`、`expected_revision`。

`key` 创建后不可改。服务端自动同步当前 Server 的 `debug` 空间；其他空间由调用方显式同步。空间软删除后其内容仍保留，可经 `--include-deleted` 读取。

列表返回各空间的 `key`、名称、描述与初始化状态；后续命令的 `--env <env-key>` 使用该 `key`。更新示例中的 `expected_revision` 替换为查询返回的当前版本号。

## 实体类型与关系类型

```sh
opencoder-cli --no-auth ontology entity-types list
opencoder-cli --no-auth ontology entity-types create --json '{"key":"component","name":"组件","description":"示例类型"}'
opencoder-cli --no-auth ontology entity-types update <type-id> --json '{"name":"组件（更新）","is_deleted":false,"expected_revision":1}'
opencoder-cli --no-auth ontology relationship-types list
opencoder-cli --no-auth ontology relationship-types create --json '{"key":"depends","name":"依赖","source_entity_type_id":"<type-id>","target_entity_type_ids":["<type-id>"]}'
opencoder-cli --no-auth ontology relationship-types update <relationship-type-id> --json '{"name":"依赖（更新）","source_entity_type_id":"<type-id>","target_entity_type_ids":["<type-id>"],"is_deleted":false,"expected_revision":1}'
```

字段约定：

- 实体类型创建体：`{"key","name","description"?}`。
- 关系类型创建体：`{"key","name","description"?,"source_entity_type_id"?,"target_entity_type_ids"}`；省略 `source_entity_type_id` 表示任意源类型。
- 两者的更新体都需要 `name`、`is_deleted`、`expected_revision`。
- `key` 创建后不可改；名称变化沿用原实体身份，不产生一个无关的新实体。

关系类型定义允许的源、目标类型组合；具体关系的两端必须匹配该定义且属于同一空间。目录归属关系类型带有 `is_directory_membership:true` 标记，由服务端内置，挂载用法见 [directories.md](directories.md)。

类型列表返回每项的 ID、`key` 与名称；创建和更新响应中的 `id` 是后续属性、Action、实体操作的依据，先读取再引用，不凭名称猜测 ID。

## 属性

属性定义挂在实体类型下，按类型维护：

```sh
opencoder-cli --no-auth ontology attributes list <type-id>
opencoder-cli --no-auth ontology attributes create <type-id> --json '{"key":"owner","name":"负责人","kind":"text","required":false}'
opencoder-cli --no-auth ontology attributes update <attribute-id> --json '{"name":"负责人（更新）","required":true,"is_deleted":false,"expected_revision":1}'
```

字段约定：

- 创建体：`{"key","name","description"?,"kind","required"?,"attribute_role"?,"storage_mode"?}`。
- 更新体：`{"name","description"?,"required","is_deleted","expected_revision","attribute_role"?,"storage_mode"?}`。

`kind` 的取值：

| 类别 | 取值 |
| --- | --- |
| 结构化类型 | `string`、`integer`、`float`、`boolean`、`datetime`、`json` |
| 文本与向量 | `text`、`vector` |

- 文本属性默认以 markdown 存储；`storage_mode` 取 `sql`、`markdown`、`nfs_path`。
- `attribute_role` 取 `custom`、`source`、`ext`，标记属性承担的角色。
- 停用的属性定义不能再写入值；已保存的历史值仍随实体保留。
- 必填属性在创建实体时应提供初始值。

属性值本身按实体写入，见 [records.md](records.md) 的「属性值与正文」；本节只维护定义。`source` 记录来源、`ext` 记录补充说明，平台生成实体的这些角色属性由同步维护，人工补充内容不被覆盖。

## Action 配置

Action 记录实体类型下读、写操作的配置：

```sh
opencoder-cli --no-auth ontology actions list <type-id>
opencoder-cli --no-auth ontology actions create <type-id> --json '{"operation_type":"read","operation":"list-components","description":"读取组件清单"}'
opencoder-cli --no-auth ontology actions update <action-id> --json '{"description":"读取组件清单（更新）","is_deleted":false,"expected_revision":1}'
```

- 创建体：`{"operation_type","operation","description"?}`。
- 更新体：`{"description"?,"is_deleted","expected_revision"}`。

这些命令维护读写操作配置，本身不执行 Action；实际执行回到原模块的定义与权限校验。

## 更新流程示例

先读后写，携带查询返回的当前版本号：

```sh
# 1. 读取对象与当前 expected_revision
opencoder-cli --no-auth ontology entity-types list --include-deleted
# 2. 提交更新（版本号替换为实际值）
opencoder-cli --no-auth ontology entity-types update <type-id> \
  --json '{"name":"组件（更名）","is_deleted":false,"expected_revision":2}'
# 3. 读回验证
opencoder-cli --no-auth ontology attributes list <type-id>
```

- 同一对象被并发修改时会返回版本冲突；重新读取最新内容后再决定是否重试。
- 软删除示例：`--json '{"name":"组件（弃用）","is_deleted":true,"expected_revision":3}'`。
- 长中文正文可写入临时 JSON 文件后使用 `--json @/absolute/path.json`，避免 shell 转义出错。

## 常见错误

- 版本冲突：更新返回冲突时重新读取对象，用新的 `expected_revision` 重试，不覆盖他人修改。
- 跨空间引用：类型、实体、关系必须同空间，混用 ID 会被服务端拒绝。
- 软删除字段缺失：软删除也是一次 `update`，仍需提供该更新体的必填字段（如 `name`、`is_deleted`、`expected_revision`）。

命令成功后用 `list` 或 `get` 读回验证关键字段；列表默认排除已删除对象，需要保留数据时加 `--include-deleted`。具体参数以 `--help` 为准。
