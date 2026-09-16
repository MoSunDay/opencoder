Commit: 071c3aca7dd59b767ea4652eed82d7589d03c226

# 存量 DAG 迁移到当前 Agent 执行契约

`eval-diagnose`、`regression-test` 是移除 DAG Runner 类型之前保存的定义。当前引擎只接受 `agent`、`wasm`；定义列表仍可读取旧记录，保存和执行会拒绝其 `runner` 步骤。此前清理保留了原定义，本次按用户要求完成显式迁移。

## 适配内容

- 两条定义的 `workflow` 步骤改为 `type=agent`，继续使用各自同名 Agent，保留名称、依赖与 3,600 秒超时；移除 `runner` 字段，补齐评测归因、回归审查及缺少输入时的处理要求。
- 输入通过通用执行接口的 `input.prompt` 提供。沿用现有 Codex harness 和模型，不恢复已退役的 Runner 接口。
- 命名 Codex profile 原先引用旧的独立认证目录，其认证副本已过期并触发刷新令牌重复使用错误。profile revision 2 改为引用现有主 Codex 目录，保持同一账号、认证槽位、模型与权限。有效凭证文件前后摘要一致，没有复制、改写或过期令牌。
- 两个 prompt 资源均更新至 v2，`how.md` 显式引用当前技能资源版本目录中的 `ed/SKILL.md`、`cr/SKILL.md`，只允许使用本次资源快照。同步刷新 Agent 元数据中的技能引用，消除旧路径读取失败及宿主全局技能替代。
- 通过现有管理 API 生效。能力库保持为空，DAG 仍恰好四条，另外两条定义与迁移前完全一致。

## 验证

| 验证项 | 证据 | 结果 |
| --- | --- | --- |
| 定义保存及更新时间 | 两条定义均从页面打开 JSON 编辑器并保存；spec 不变、创建时间不变、更新时间递增 | 通过 |
| 目录与页面 | 四条 DAG 均显示更新时间，绝对时间提示正常，能力库 0 条，浏览器无脚本错误 | 通过 |
| 评测归因真实执行 | `dag-agent-adapt-v2-ed-12bece6c7b4a`：读取本地合成 case，识别 nominal success 中的 upload 失败并保留 `case-007` | done |
| 回归审查真实执行 | `dag-agent-adapt-v2-reg-0c0eacc12dd2`：读取合成 diff，通过 Node 在内存中复现省略 limit 后结果由 `[1,2,3]` 变为 `[]` | done |
| 工具与资源证据 | 最终两次执行各 4 次真实工具调用，0 次工具失败；读取当前固定技能快照，没有使用宿主全局技能替代 | 通过 |

验证输入仅为本地合成样例；没有提交真实业务任务、建单、发布或发消息。原定义、profile、prompt 资源与各轮执行回执均保存在服务器本地 `/var/lib/opencoder-platform/backups/dag-agent-adaptation-20260916-135703/`，最终回执为该目录下的 `completion.json`。

本次为线上定义、资源和执行配置的适配，未变更应用代码，验证直接覆盖运行中的原生 Agent 路径。模块职责与产品接口未变化，保留既有稳定记忆及顶层索引。

## 相关

- [前置清理与更新时间修复](dag-definition-timestamps.md)
- [Agent 平台定义管理](../../agent-platform/index.md)
- [当前 DAG 步骤类型](../../../docs/registered-runners.md)
