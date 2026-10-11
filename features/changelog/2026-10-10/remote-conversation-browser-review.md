Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# 远端会话的真实浏览器复验

当前 Web 的 Agent 与 Operator 首轮提交均可复现 504，随后读取返回 `execution not found`。节点在受理前复制整个资源池，实际 NFS 中大量未被 Agent 引用的资源使复制超过请求时限。

普通会话改为固定全部已注册 Agent 卡及其引用资源，保留会话内切换身份的能力；不再读取未使用资源池。Agent 元信息与依赖选择使用同一份内容，复制任务全部结束并完成落盘后才发布快照。DAG 继续只固定所选 Agent 的依赖。

复核同时覆盖已存在会话行、但仍在排队或执行时的事件订阅，以及远端接收回执的执行归属和请求身份校验。Server 仍须收到节点持久接收确认后才发布会话执行索引。

正式页面复验进一步发现：续聊在等待宿主机名额时，事件接口因没有活跃 drain 而提前返回结束，页面遗漏随后完成的回复。事件接口现在将排队中的普通会话视为未结束；历史列表在读取详情前排除内部 Agent 索引，并限制为最多 16 个并发读取，保持原有顺序和错误展示。

## 测试覆盖

| 功能 | 测试或验收入口 | 文件 |
| --- | --- | --- |
| 保留可切换身份，忽略未引用的损坏资源池 | `catalog_keeps_switchable_identities_and_skips_unreferenced_pools` | [snapshot/tests.rs](../../../crates/agents/src/snapshot/tests.rs) |
| 缺失必需依赖时不得发布快照 | `catalog_rejects_missing_referenced_dependency_without_publishing` | [snapshot/tests.rs](../../../crates/agents/src/snapshot/tests.rs) |
| 两种模式实际加载 Server 身份、Skill、Tool，重启续聊 | `operator_uses_server_identity_skills_tools_and_resumes_on_host`、`agent_uses_server_identity_skills_tools_and_resumes_on_host` | [remote_identity.rs](../../../tests/operator_e2e/remote_identity.rs) |
| 已保存会话排队与运行期间保持订阅 | `agent_is_readable_while_waiting_for_a_host_slot`、`operator_is_readable_while_waiting_for_a_host_slot` | [conversations/main.rs](../../../crates/worker/tests/conversations/main.rs) |
| 续聊等待名额时保持事件流，执行完成后正常结束 | `queued_continuations_keep_agent_and_operator_event_streams_open` | [conversations/main.rs](../../../crates/worker/tests/conversations/main.rs) |
| 历史列表不读取内部子会话，保留详情过滤与错误行 | `agent_dialogs_skip_internal_indexes_before_querying_nodes`、`dialogs_rows_are_happy_degraded_and_node_scoped` | [sessions_compat_extra.rs](../../../crates/control/tests/e2e/sessions_compat_extra.rs) |
| 首轮、续聊、空流恢复及重新打开运行中的会话 | `conversation`，分别执行 Agent 和 Operator | [chat.js](../../../scripts/acceptance/ui/scenarios/chat.js) |
| 正式页面选择节点与身份，真实模型、工具结果、续聊和重新打开 | `browser.js --base … --token-file … --node … --agent … --output …` | [browser.js](../../../scripts/acceptance/conversations/browser.js) |

## 已完成的验证

证据根目录：`/data00/root-storage-20261008/tree/tmp/opencoder-remote-review-6affi3_r`。

- 当前线上旧版本的两种模式均复现失败，见 `live-browser/attempt2/result.json` 及失败截图。
- 三个原始超时请求随后在远端完成，返回了真实工作目录；浏览器的首次受理仍然失败。它们均已空闲，不阻塞发布，见 `live-browser/late-completions.json`。
- 同一实际只读 NFS 和本地数据盘对比：全池复制 97.79 秒，按注册身份依赖复制 10.09 秒；Agent 卡均为 25 张，资源池由 324 个降至 53 个。见 `resource-copy-comparison.json`。
- 修复后的独立 Server、Node 使用真实线上资源和真实模型，浏览器完成两种模式首次提交、工具执行、续聊与重新打开；受理分别耗时 19.03 秒和 27.19 秒。脱敏回执见 `isolated-browser-verified.json`，截图见 `live-model/`。该结果与正式入口验收分开记录。
- `cargo test --workspace --test operator_e2e --test conversations --test conversation_acceptance --test project_dispatch --no-fail-fast -j4 -- --test-threads=4`：23 项通过，见 `catalog-regressions-current.log`。
- 9 个相关前端测试文件：88 项通过，见 `release-spa-targeted.log`。覆盖会话初始化、结束补读、Agent 配置和直接相关导航；只记录该范围通过。
- 成套隔离二进制的浏览器回归通过，两种模式均完成首轮、续聊、空事件流恢复及重新打开运行中会话，见 `release-ui-evidence/chat-ui.json`。
- 真实 runc 升级、回滚及连续任务演练通过，观察持续 900.00 秒、完成 58 个观察任务；切换期间 44 次受理的最大耗时 0.218 秒。见 `release-smooth-evidence/result.json`。该演练使用私有 tmpfs，用于功能与竞态验证；真实磁盘资源复制和正式入口验收单独记录。
- 对共享 DAG 资源复制执行 Rust 全量检查，原始结果为 5758 passed / 2 failed / 8 ignored。两个失败均由隔离候选混入另一项 Operator 提示词改动造成；候选恢复其基线提示词后，重建配套二进制并重跑完整受影响目标，22 项会话测试及 9 项 Worker 平台测试全部通过。原始失败保留在 `release-rust-tests.log`；修正范围见 `release-scope-correction.json`，汇总见 `independent-validation.json`。共享工作区和正式候选保留它们完整的新版提示词实现。
- Clippy、最终格式检查、成套构建及最终 workspace 构建通过。正式发布包 `3e870567` 的两种会话、派生和身份缺失链路另有 5 项端到端测试通过，见 `/var/tmp/opencoder-deploy-20261010/final-e2e-result.json`。
- 正式版本 `3e870567` 的浏览器首轮受理耗时分别为 2.786 秒和 4.116 秒，两种模式均实际执行 `pwd`；Agent 完成续聊与重新打开，但 Operator 续聊回复遗漏，因此整轮判为失败，见 `live-public-browser/result.json` 和 `operator-failure.png`。远端保存的回复及事件确认了排队空档提前结束订阅的问题。
- 排队事件新增断言在旧实现中两项均失败（`queued-events-red.log`）；修复后，包含实际排队续聊的 5 项 Worker 会话回归全部通过（`queued-events-final.log`）。历史列表 6 项定向回归通过（`dialog-history-green.log`）。新增续聊用例显式指定标题，避免测试模型因自动生成标题多调用一次而等待错误的通知。
- 原正式 Agent 历史列表读取 326 条可见会话耗时 175.88 秒，见 `live-public-browser/dialogs-timing.json`；列表读取改为保持顺序的有界并发，修复后延迟由正式入口复验单独记录。
- 修复包叠加到已上线的项目全局标签版本 `e3ca4fab`，发布为 `b490f40b`，保留其改动。会话修复文件摘要未变，复用候选 `72987a75` 已通过的 5 项 Worker、6 项 Control 回归；最终发布包另跑 5 项真实 Server/Agent 端到端测试，全部通过。见 `followup-rebase.json`、`followup-verified.json`。
- 正式入口 `http://127.0.0.1:18081`、节点 `human-os-02` 的浏览器验收通过：Agent 和 Operator 均选择 Server 的 `operator` 身份，首轮提交分别耗时 4.023 秒、2.886 秒，实际执行一次 `pwd`，完成续聊并重新打开同一会话。请求与持久会话身份一致，浏览器无异常、无失败 HTTP 响应。见 `final-live/browser/result.json`、`final-live/tool-verification.json`、`final-live/identity-verification.json` 及两种模式的 `*-reopened.png`。
- 新版本历史列表返回 336 条会话耗时 7.257 秒，见 `final-live/history-timing.json`。
- 正式版本 `b490f40b` 完成 900.97 秒上线观察，179 次入口健康、就绪及所选执行节点检查全部通过，最大成功采样间隔 6.22 秒。该观察为只读健康检查，不计作生产负载测试。见 `final-live/live-observation.json`；最终汇总回执为 `accepted.json`，发布交付回执为 `followup-delivery.json`。
