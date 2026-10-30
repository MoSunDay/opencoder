# 能力目录与同步

能力目录登记平台全部公开能力，同步把原模块定义整理为可查询的实体与关系。以下示例适用于鉴权已关闭的 Server；占位地址与 `<...>` ID 需替换为实际值。

## 身份与能力查询

先确认身份与 Ontology 权限，再查询能力目录：

```sh
opencoder-cli --no-auth ontology session
opencoder-cli --no-auth ontology capabilities list
opencoder-cli --no-auth ontology capabilities list --kind dag --query example
opencoder-cli --no-auth ontology capabilities list --offset 0 --limit 100
opencoder-cli --no-auth ontology capabilities get <entity-id>
```

能力列表返回 `items` 和筛选后的 `total`；每项包含稳定的 Ontology 实体 ID、`capability` 描述与停用标记。`capabilities get` 按 Ontology 实体 ID 读取单个能力详情。

`session` 返回当前身份与 Ontology 读写权限；后续查询和同步命令按该结果判断可用范围。身份不符或权限缺失时先与用户确认目标服务器，不假设可以越权操作。

常用筛选参数：

- `--kind` 按能力类型过滤，例如 `dag`。
- `--query` 按关键词过滤名称与描述。
- `--offset` 与 `--limit` 分页，`total` 是筛选后的总数。
- `--include-deleted` 包含已停用的能力登记。

能力描述包含来源标识、类型、版本、输入输出要求、CLI/API 入口及明确依赖。查找和理解能力从这里开始，再沿记录的入口进入原模块；能力出现在目录中只表示已登记，能否运行仍由原模块校验。

## 同步状态与差异

```sh
opencoder-cli --no-auth ontology sync status
opencoder-cli --no-auth ontology sync check
opencoder-cli --no-auth ontology sync run
```

`sync status` 的状态包含 `not_initialized`、`running`、`ready`、`error`，并提供最后成功时间和错误信息；`issues` 列出无法解析的引用。

`sync check` 只读比较来源与已同步内容，包括平台生成字段是否偏离：一致退出 0，存在差异退出 4。该命令不修改任何数据。

`sync run` 等待一次完整同步，返回新增、更新、停用及修复数量；完成后再次 `sync check` 确认一致。

补充说明：

- 来源读取或写入失败时保留最后成功内容并持续重试，不产生半更新的目录。
- 读取能力沿用 Ontology 读权限，手动同步沿用写权限。
- 服务端自动同步当前 Server 的 `debug` 空间；其他空间需先显式指定 `--env <env-key>`，再手动执行 `sync run`。
- 自动同步可能有短暂延迟，不改变原模块的保存回执。

## 差异修复流程

1. `sync check` 返回退出码 4，确认存在差异。
2. 执行 `sync run`，等待一次完整同步结束。
3. 再次 `sync check`，确认退出码为 0。

差异处理约定：

- 能力内容偏差通过原模块修改后重新同步；本目录命令不直接改写平台生成字段。
- 同步只维护自身生成的数据；人工补充说明、自定义属性及关系不被覆盖。
- 来源读取失败不能当成空目录；完整读取成功后才停用不再存在的能力。
- 生产同步遵循用户授权；开发测试只连接临时服务。

## 验证入口

能力查询和同步 API 位于 `/api/ontology/envs/:env`：

- `GET /capabilities`、`GET /capabilities/:id` 查询能力。
- `GET /sync` 查询同步状态，`GET /sync/check` 只读比较。
- `POST /sync` 执行并等待一次同步。

CLI 封装以上接口，优先使用 `opencoder-cli ontology`；直接调用 API 时沿用相同的占位地址与鉴权方式。
