Commit: (working-tree, 基于 b465f440381bd009dc9bd3a8192ad88eab44cede)

# 仓库本地记忆收敛为纯索引形态

按「记忆只是索引，代码是最终事实，解释交给代码」原则，26 份非 changelog 记忆文件（agents.md、agents/*/index.md ×22、features 四索引）统一为固定结构：Commit 基线 + 一行职责 + 关键路径（路径/符号 + 单子句硬契约）+ 边界 + 相关链接。删除叙述段落、how/why 解释、历史修复故事与 CLI flag 全枚举；净减 449 行（+680/−1129），单文件 ≤45 行。

全量漂移核查修正 15 项过期事实：auth_sig 消失、llm 读超时 1800s→600s、store schema 22→23、TsMirrorStore 迁至 tui、session 入口迁 runner/entry、internal-python-step 退役为 InternalProcessSupervisor、PeerBridge 与 system 团队执行退役、Python 执行器双面下线、dag-runtime 链接面更正（worker/agent 链接、仅 server 不链接）、project overview 实际位于 store 层、harness 分层为 core/session 两处等。26 文件 Commit 基线统一为 b465f440。

## 测试覆盖

仅文档变更（记忆树 .md），零代码改动，按用户指示免测提交。文档侧验证：

| 功能 | 测试名 / 验证 | 文件 |
| --- | --- | --- |
| 断言真实性 | 26 文件逐路径/符号 grep 核验；session/web/control 二轮行级证据复核（修正 catalog/executions 归属 1 处、web `/seq` 全路径与 `client_override` 接缝 2 处） | 记忆树全量 |
| 链接有效性 | 相对链接解析全部通过（含跨树 docs/ 链接） | agents.md、agents/*、features/* |
| 结构门禁 | 单文件 ≤400 行（实测 ≤45）、目录 ≤10 md、kebab-case、基线 26/26 统一 b465f440 | 记忆树全量 |
