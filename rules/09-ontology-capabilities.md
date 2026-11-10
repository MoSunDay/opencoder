Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# 规则 09：Ontology 能力目录与 CLI

## 定位与使用顺序

Ontology 是 OpenCoder 全部平台能力的统一查询和组织入口，用于回答：现在有哪些能力、各自做什么、需要什么输入、产出什么、依赖谁、从哪里调用。查找、理解和组织现有能力时，先从 Ontology 的能力目录、实体目录和图谱开始，再沿记录的 CLI/API 入口进入原模块。

原模块负责能力定义、权限校验、派发和执行，Ontology 保存可查询的公开描述及其关系。执行时仍读取原模块的定义和状态；能力出现在 Ontology 中只表示已被登记，能否运行仍由原模块校验。

平台功能入口由共享清单登记；Agent、Operator、Team、DAG、TODO 工作流和 Brain 计划的公开能力描述由服务端从原生定义同步。能力身份使用稳定来源标识，名称变化不应产生一个无关的新实体。

## 对象如何组织

| 对象 | 含义与用途 |
| --- | --- |
| 实体类型 | 定义一类对象及其属性、Action 配置 |
| 实体 | 表示一个具体对象；能力、目录都通过实体承载 |
| 属性 | 保存实体的结构化字段或正文；`source` 记录来源，`ext` 记录补充说明 |
| 关系类型与关系 | 定义连接的含义及允许的源、目标类型，再记录具体实体之间的连接 |
| 实体目录 | 通过目录归属关系组织目录树和实体成员；目录节点最多有一个父目录，普通实体可同时归属多个目录，同一实体在同一目录只能有一条归属 |
| 图谱 | 根据实体、关系、中心和上下游深度查询关联范围 |
| 切面 | 保存一组图谱观察条件，包括类型、关系、中心和默认深度，可反复查询、调整和使用 |
| Action | 记录实体类型下读、写操作的配置，维护配置的命令本身不触发执行 |
| 数据空间 | 隔离一组类型、实体、关系和切面，引用对象必须属于同一空间 |

目录负责层级归属，普通关系负责表达依赖等连接，切面负责保存观察范围。组织能力时应分别维护这三部分，关系必须有明确来源，不能仅凭名称相似推断依赖。

## 实体目录与挂载

实体目录的“挂载”指将实体归入目录：新实体创建时传 `directory_id`，已有实体通过创建目录归属关系挂载；归属关系从成员指向父目录。目录节点必须保持单一父目录；普通实体可挂载到多个目录，追加目录时直接创建新的归属关系，无需先解除已有归属。服务端拒绝同一实体对同一目录的重复归属、目录节点的第二个父目录、目录循环、移动根目录及删除非空目录。

解除挂载即软删除对应归属关系，多个归属可并存、互不影响；移出某个目录只需软删除对应归属。子目录换父目录使用 `directories move`；空目录通过 `directories update` 的 `is_deleted:true` 删除。查询目录成员时，在返回关系中筛选目录归属类型且 `target_entity_id` 为当前目录的记录，`source_entity_id` 就是成员 ID，再用 `entities get` 读取详情；目录自身的父级关系方向相反，不属于该目录的成员。这些记录表示直接成员，查询子目录的成员需继续遍历目录树。

## 自动维护与人工维护

- 平台生成的能力基本信息、定义属性、来源、目录归属和固定切面由同步维护，修改时应回到原模块。
- 人工补充说明、自定义属性和自定义关系可以通过 Ontology 维护，后续同步必须保留。
- 普通自建类型、实体、关系、目录和切面通过 Ontology 的增删改查维护，更新使用当前版本，删除沿用软删除约定。
- 来源中的能力被移除后，同步停用对应登记并保留历史；来源读取失败时保留最后一次成功内容，不能把失败解释为能力被删除。
- 同步只保存公开能力信息和明确依赖；凭据、私有配置、项目实例、任务状态和执行记录由原模块保存。不得修改已有鉴权数据。

服务端启动、成功修改能力定义后触发同步，并每 30 秒检查一次。自动同步目标为 `debug`，其他数据空间显式同步。保存成功与同步完成是两个状态，调用方需要立即确认一致性时，使用 `sync status`、`sync check` 和 `sync run`。

托管发布中只有当前版本的当前 Server 实例可以写入同步结果和状态。候选预热、切换和回滚准备阶段不得改写共享目录，`sync check` 仍可检查差异。发布记录与同步提交共用文件锁，写权限按版本、端口和阶段核验；记录缺失或不匹配时拒绝写入。独立 Server 不依赖发布记录。

## 能力变更流程

所有修改都必须检查是否影响 Ontology。新增、修改或删除用户可见能力、CLI/API 入口、使用方式、输入输出要求或依赖时，必须同时维护对应登记、说明、关系和测试，不能以“后续补录”作为完成状态。纯内部重构如果没有改变这些内容，核对现有登记仍准确即可，无需额外创建实体。

按以下顺序完成能力变更：

1. 查询已有能力、来源和关系，确认对应的原模块以及已有实体身份。
2. 修改原模块；同时维护能力描述、入口、输入输出要求和明确依赖。
3. 新增平台入口时更新 [共享清单](../crates/core/src/ontology/platform.rs)；新增原生来源或修改转换规则时更新 [同步模块](../crates/control/src/ontology_catalog/mod.rs)。
4. 对受影响链路验证同步和 CLI 行为，确认新增可查、修改可见、移除会停用，人工内容保持不变。
5. 更新相关规则、使用说明和必要的仓库记忆，记录实际验证范围。

- 新增来源必须覆盖完整读取、更新、删除、读取失败和恢复。
- 关系只取自定义中的明确引用，不通过名称或自然语言猜测。发现缺失依赖时返回问题，不能静默省略。
- 自动字段通过原模块修改。同步只维护自身生成的数据；人工补充说明、自定义属性及关系不被覆盖。
- 来源读取失败不能当成空目录。完整读取成功后才允许停用不再存在的能力；重试必须保持实体身份、人工内容和不可变正文。

## CLI 操作约定

CLI 使用已有 Server 连接配置和鉴权。默认数据空间为 `debug`，通过 `opencoder-cli ontology --env <env-key> ...` 指定其他空间。服务端自动同步当前 Server 的 `debug`；其他空间由调用方显式同步。

- 写入体使用 `--json '<JSON>'` 或 `--json @file.json`，字段与原 API 相同。
- 更新携带查询返回的当前 `expected_revision`；发生版本冲突时重新读取，不覆盖他人修改。
- 新建实体携带 UUID 格式的 `request_id`，同一次创建重试保持 ID 和内容不变；`source`、`ext` 必填。
- 除切面有独立 `delete` 命令外，实体、关系、类型、属性、Action、目录、保鲜策略和环境均通过 `update` 设置 `is_deleted:true` 软删除，仍需提供对应更新体的必填字段。
- 普通列表排除已删除对象；`--include-deleted` 可读取保留的数据。Action 命令维护操作配置，本身不执行 Action。

成功响应写入 stdout，失败写入 stderr。退出码：0 成功，1 传输失败，2 鉴权或权限失败，4 服务端拒绝或同步检查发现差异。具体命令参数可用 `--help` 查询。

### 能力目录与同步命令

```sh
opencoder-cli ontology session
opencoder-cli ontology capabilities list --kind dag --query example
opencoder-cli ontology capabilities get <entity-id>
opencoder-cli ontology sync status
opencoder-cli ontology sync check
opencoder-cli ontology sync run
```

能力列表返回 `items` 和筛选后的 `total`，支持 `--offset`、`--limit`、`--include-deleted`。每项包含稳定的 Ontology 实体 ID、`capability` 与停用标记。能力描述包含来源标识、类型、版本、输入输出要求、CLI/API 入口及明确依赖。

`sync check` 只读比较来源与已同步内容，包括平台生成字段是否偏离；一致返回 0，存在差异返回 4。`sync run` 等待一次完整同步，返回新增、更新、停用及修复数量；同步后再次 `sync check` 确认一致。同步状态包含 `not_initialized`、`running`、`ready`、`error`、最后成功时间和错误信息；`issues` 列出无法解析的引用。来源读取或写入失败时保留最后成功内容，后台持续重试。自动同步可能有短暂延迟，不改变原模块的保存回执。

### 命令清单

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
| `freshness` | `policies` 的 `list`、`get`、`create`、`update`；`runs` 的 `list`、`get`、`trigger`；`reviews` 的 `list`、`create` |
| `nfs` | `status`、`set`；管理正文只读导出 |

### 实体、关系与切面

以下 `<...>` 均替换为实际 ID，版本号示例替换为查询返回的当前版本。

```sh
opencoder-cli ontology entity-types create --json '{"key":"component","name":"组件"}'
opencoder-cli ontology entities create --json '{
  "request_id":"<request-uuid>","entity_type_id":"<type-id>",
  "name":"组件 A","source":"来源说明","ext":"补充说明"}'
opencoder-cli ontology entities update <entity-id> --json '{
  "name":"组件 A（更新）","description":"说明","is_deleted":false,"expected_revision":1}'
opencoder-cli ontology relationship-types create --json '{
  "key":"depends","name":"依赖","source_entity_type_id":"<type-id>",
  "target_entity_type_ids":["<type-id>"]}'
opencoder-cli ontology relationships create --json '{
  "relationship_type_id":"<relationship-type-id>",
  "source_entity_id":"<source-id>","target_entity_id":"<target-id>"}'
opencoder-cli ontology relationships list --anchor <entity-id>
opencoder-cli ontology graph --center <entity-id> \
  --entity-type-id <type-id> --relationship-type-id <relationship-type-id> \
  --upstream-depth 1 --downstream-depth 2
opencoder-cli ontology aspects create --json '{
  "aspect_key":"component-dependencies","name":"组件依赖",
  "entity_type_ids":["<type-id>"],"relationship_type_ids":["<relationship-type-id>"],
  "default_center_ids":["<entity-id>"],
  "default_upstream_depth":1,"default_downstream_depth":2}'
```

软删除统一通过 `update` 设置 `is_deleted:true`（仍需必填字段），切面使用独立删除命令：

```sh
opencoder-cli ontology entities update <entity-id> --json '{
  "name":"组件 A（更新）","description":"说明","is_deleted":true,"expected_revision":2}'
opencoder-cli ontology aspects delete <aspect-id> --expected-revision 2
opencoder-cli ontology entities list --include-deleted
```

`relationships list --anchor` 返回该实体作为源或目标的关系，不传 `--anchor` 则返回全部关系。切面保存的条件通过 `aspects get` 读取后传给 `graph` 查询；当前 `graph` 没有直接接收切面 ID 的参数。

### 实体目录操作

```sh
opencoder-cli ontology directories tree
opencoder-cli ontology directories create --json '{"name":"组件目录"}'
opencoder-cli ontology directories create --json '{"name":"子目录","parent_id":"<directory-id>"}'
opencoder-cli ontology directories move <child-directory-id> \
  --json '{"parent_id":"<new-parent-directory-id>"}'
```

省略 `parent_id` 时创建在根目录下，根目录 ID 来自 `directories tree` 的 `root_id`。新建实体时增加 `directory_id`，创建成功后即归入该目录；已有实体挂载时，先从 `relationship-types list` 找到 `is_directory_membership:true` 的关系类型，再创建从实体指向目录的归属关系：

```sh
opencoder-cli ontology relationships create --json '{
  "relationship_type_id":"<directory-membership-type-id>",
  "source_entity_id":"<entity-id>","target_entity_id":"<directory-id>"}'
opencoder-cli ontology relationships list --anchor <directory-id>
```

解除挂载用 `relationships update <membership-relationship-id>` 设置 `is_deleted:true` 软删除该归属。

### API 与验证入口

能力查询和同步 API 位于 `/api/ontology/envs/:env`：`GET /capabilities`、`GET /capabilities/:id` 查询能力；`GET /sync` 查询状态，`GET /sync/check` 只读比较，`POST /sync` 执行并等待一次同步。读取沿用 Ontology 读权限，手动同步沿用写权限。已有实体、图谱和正文 API 可访问同步生成的数据。

CLI 实现见 [命令模块](../crates/ctl/src/cmd/ontology/mod.rs)。命令映射见 [解析测试](../crates/ctl/tests/parse_ontology.rs)，真实 CLI 与服务端操作见 [集成测试](../crates/ctl/tests/ontology_server.rs)及其 [目录](../crates/ctl/tests/ontology_server/directories.rs)、[实体、关系与切面](../crates/ctl/tests/ontology_server/records.rs)、[类型与环境](../crates/ctl/tests/ontology_server/schema.rs)用例。按本次改动范围选择测试，不默认运行全量测试。

## 验收

- 公共 CLI 域和控制面 API 路径必须通过能力登记检查。修改某个命令或入口时，同时核对其说明仍准确。
- 对本次变更执行 `ontology sync check` 或隔离测试中的等价检查；有差异时同步并再次核对。开发测试只连接临时服务，生产同步遵循用户授权。
- CLI 必须持续覆盖类型、属性、Action、实体、关系、切面、目录和数据空间的增删改查，以及实体详情、关联关系、图谱范围查询和实体目录挂载。切面提供独立删除命令，其余对象通过更新 `is_deleted` 软删除。
- 目录相关修改必须验证新实体归属、已有实体挂载、解除与重新挂载、子目录移动、成员查询，以及实体对同一目录的重复归属、目录节点的第二个父目录、循环和非空目录删除的拒绝。
- 将同步行为、错误情况及实际测试结果写入相关功能说明、模块记忆和当日变更记录。范围遵循 [定向回归规则](02-regression-gate.md)。

实现边界与页面行为见 [Ontology 功能说明](../features/platform/ontology/index.md)。
