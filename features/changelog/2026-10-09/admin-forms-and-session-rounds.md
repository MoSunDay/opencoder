Commit: ee4879894d929df76afaced879246d482dd3aeff

# 后台管理表单与会话轮次

用户、Token 和节点列表共用搜索、筛选与表格布局。创建和编辑操作移入弹窗，提交期间禁止重复操作，失败保留草稿；Token 只在签发后展示一次。用户目录读取失败时禁用签发与撤销，节点 CPU 未知时显示未知负载。

Team 的队长决策和成员回答不触发任务完成后的本地记忆维护，保持每个中间发言对应的模型轮次。Codex 旧执行配置可显式迁移空的 `auth_slot` 字段；迁移默认预览，写入前保存可校验的回退文件，非空字段和记录冲突均拒绝写入。

Bash 单元测试的工具上下文持有独立临时配置目录，避免开发机登录脚本的耗时和输出影响命令完成、退出码及后台输出上限断言，保留原有一秒前台等待门槛。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 弹窗确认、角色保存与重复提交 | `saves a role only after confirmation and prevents duplicate submission` | [access.dom.test.jsx](../../../crates/web/spa/src/admin/access.dom.test.jsx) |
| Token 签发与一次展示 | `issues a token from a dialog and removes plaintext after acknowledgment` | [tokens.dom.test.jsx](../../../crates/web/spa/src/admin/tokens.dom.test.jsx) |
| 目录失败时禁止操作 | `disables issuance and revocation when the user directory fails` | 同上 |
| 状态边界与列表响应校验 | `classifies token expiry at the boundary and gives revocation precedence`、`distinguishes empty directories from malformed responses` | [model.test.js](../../../crates/web/spa/src/admin/model.test.js) |
| 节点搜索、状态与操作可用性 | `combines node search and online status while preserving action availability` | [nodes.dom.test.jsx](../../../crates/web/spa/src/admin/nodes.dom.test.jsx) |
| Team 多轮讨论不额外调用模型 | `team_multiround_consensus_runs_alignment_subturn_and_next_round_hint` | [team_multiround_consensus.rs](../../../crates/worker/tests/platform/team_multiround_consensus.rs) |
| 配置迁移、重试、冲突与回退 | `test_preview_apply_retry_restore_preserves_request_and_script` 等 5 项 | [test_codex_startup.py](../../../scripts/platform/migrations/test_codex_startup.py) |
| Bash 命令完成、退出码与后台输出上限 | `bash_normal_completion`、`bash_failure_appends_exit_code`、`bash_short_command_completes_normally`、`background_output_overflow_stops_process_and_caps_file` | [completion.rs](../../../crates/session/src/tools/bash/tests/completion.rs) |

全量回归：Rust workspace 5,747 项、SPA 1,130 项、Python 234 项、JavaScript 验收辅助测试 16 项通过。格式检查、Clippy、构建、SPA 类型与产物检查通过；全站浏览器、四种屏宽、TUI、NFS、runc、旧 Runtime，以及升级回滚和 15 分钟观察均通过。

实现索引：[Web](../../../agents/web/index.md)、[Worker](../../../agents/worker/index.md)。配置迁移见 [Codex 启动脚本](../../../docs/codex-startup-script.md)，调度和资源快照见[定时调度](schedule-effective-time.md)。
