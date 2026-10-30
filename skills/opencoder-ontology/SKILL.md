---
name: opencoder-ontology
description: 使用 opencoder-cli ontology 查询和维护 OpenCoder 的能力目录、数据空间、类型、实体、关系、切面、实体目录、图谱与向量；执行仍由原模块校验，本目录不发起执行。
---

# OpenCoder Ontology 管理

使用 `opencoder-cli ontology` 调用现有 Server API，不直接写数据库。Ontology 是平台能力的统一查询与组织入口；查找、理解和组织能力先查能力目录与图谱，再沿记录的 CLI/API 入口进入原模块。

目录负责层级归属，普通关系表达依赖等连接，切面保存可复用的图谱观测条件；数据空间隔离一组对象，本 Skill 的命令不发起执行。

## 连接

先检查 `opencoder-cli --help` 和连接配置，使用用户指定的服务器或已有连接文件。以下地址为占位示例；可用 `--client-config /path/to/client.json` 指定独立连接文件。

```sh
opencoder-cli --server https://opencoder.example --no-auth ontology session
opencoder-cli --server https://opencoder.example --no-auth ontology capabilities list
```

`/api/auth/status` 无需登录，返回 `enabled`。关闭时使用 `--no-auth` 或连接文件中的 `"auth_enabled":false`；开启时使用既有 `--token-file` 或环境变量 `OPENCODER_SERVER_TOKEN`，不创建、轮换或过期任何 Token。不要把凭证写进 Skill、Git、命令示例或报告。

若返回 401/403，核对连接和已授权身份；`--no-auth` 只省略请求头，不能关闭服务端鉴权。

## 从能力目录起步

每次使用按以下顺序确认上下文：

1. `ontology session` 确认当前身份与 Ontology 权限。
2. `ontology capabilities list/get` 查询能力：描述包含来源标识、类型、版本、输入输出要求、CLI/API 入口及明确依赖。
3. `ontology sync status` / `sync check` 确认能力目录新鲜度；发现差异时的修复流程见 [references/capabilities-sync.md](references/capabilities-sync.md)。

## 命令分组

| 任务 | 参考文档 |
| --- | --- |
| 能力目录查询与同步 | [references/capabilities-sync.md](references/capabilities-sync.md) |
| 数据空间、实体类型、关系类型、属性与 Action | [references/schema.md](references/schema.md) |
| 实体、属性值、关系、图谱、切面与向量 | [references/records.md](references/records.md) |
| 实体目录与挂载 | [references/directories.md](references/directories.md) |

具体命令参数用 `--help` 查询，分组只列常用入口。

## 核心约定

- 数据空间隔离：`--env` 默认 `debug`，其他空间用 `opencoder-cli ontology --env <env-key>` 显式指定；引用对象必须属于同一空间。服务端自动同步当前 Server 的 `debug`，其他空间由调用方显式同步。
- 乐观锁：更新携带查询返回的当前 `expected_revision`；版本冲突时重新读取，不覆盖他人修改。
- 软删除：除切面有独立 `delete --expected-revision` 外，实体、关系、类型、属性、Action、目录和环境都通过 `update` 设置 `is_deleted:true` 软删除，仍需提供更新体必填字段；`--include-deleted` 读取保留数据。
- 实体创建携带 UUID 格式 `request_id`，同一次创建重试保持 ID 和内容不变；`source`、`ext` 必填非空。
- 平台生成字段：`management.managed_by:"platform-catalog"` 的实体由同步维护，`management.readonly_attributes` 列出的属性不可直接修改；改动通过原模块完成，人工补充的自定义属性与关系在后续同步中保留。
- `graph` 不接收切面 ID：先用 `aspects get` 读取观测条件，再传给 `graph` 的对应 flags。
- Action 命令只维护操作配置，本身不执行 Action。
- 退出码：0 成功；1 传输失败；2 鉴权或权限失败；4 服务端拒绝或 `sync check` 发现差异。

成功响应写入 stdout，失败写入 stderr。命令成功后读取对象验证关键字段；只重试可确认结果的请求，创建超时按相同 `request_id` 幂等重试，不把局部成功写成全部成功。
