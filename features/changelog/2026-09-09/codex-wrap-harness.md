Commit: be76fc1086cbf0d928c1d1e03ad5470563fd86df

# CLI 与 Server Web 的 Codex Harness

新增 `opencoder --wrap codex --cmd "需求" --envs KEY=VALUE`。Server Web 的 Agent 配置、启动弹窗和全部执行入口可以选择 OpenCoder / Codex；内置与自定义 Agent 都有默认 Harness，已有会话保持启动时的选择。

共享 Session 运行时调用节点上的 Codex 二进制，解析 exec JSONL 为已有消息与事件，复用 Steps / Thinking / Function call / Say 展示。需求走 stdin，环境按字面值传递。私有会话状态保存 Codex thread、fork、输入检查点和执行状态；公开执行详情隐藏环境变量值。schema 22 以可重复的增量迁移增加一列，旧会话继续使用原生执行器。

Agent 引用的 prompt、skills、tools、memory 物化到会话工作区，保留版本布局与工具可执行权限。续聊复用固定资源；Codex 使用自身配置、认证与权限。Linux CLI 和 Node 通过既有 supervisor 回收进程树，queue / steer / cancel 使用共享会话入口。损坏流、异常退出、未知提交状态和不兼容的原生控制操作直接报告错误。

Project 通过惰性客户端支持无原生凭据的 Plan/Execute；资源身份排除物化路径并包含 Harness，连续执行恢复原会话。Codex 交付清单只在 Execute 提供，完成后登记不可变文件；运行保留 Harness、thread 与模型来源。Project 预检独立检查引用存在性及本次实际执行器凭据。Fleet 升级为 v5，旧/新节点混接在注册前失败；Server 默认输出控制面错误。执行详情在移动端全宽展示并换行长 ID。

## 测试覆盖

| 功能 | 测试名 / 验收脚本 | 文件 |
| --- | --- | --- |
| CLI 参数、环境变量字面值与校验 | `harness_cli` 集成测试；`environment_preserves_payload_and_rejects_invalid_pairs` | `crates/local/tests/harness_cli.rs`、`crates/core/src/harness/mod.rs` |
| tmux 转交 Harness 与环境参数 | `tmux_passes_agent_harness_and_literal_environment_to_tui` | `crates/local/src/ts/actions_tests.rs` |
| JSONL 累计文本、工具类型和异常协议 | `harness_decode` 集成测试 | `crates/session/tests/harness_decode.rs` |
| 真实子进程、持久化、resume、fork | `codex_binary_stream_persistence_resume_and_fork` | `crates/session/tests/harness_codex.rs` |
| 资源文件与可执行工具、固定版本恢复 | `codex_reads_pinned_agent_files_and_executable_tools` | 同上 |
| 损坏流、缺少终止帧、取消与 steer / queue | `codex_malformed_stream_and_missing_terminal_fail`、`codex_cancel_reaps_descendants_and_closes_open_tools`、`codex_steer_interrupts_then_resumes_and_drains_queue` | 同上 |
| Agent 默认值、旧会话、未知提交与清空上下文 | `agent_default_applies_to_new_sessions_and_legacy_history_stays_native`、`unknown_submission_is_not_repeated_or_forked`、`clear_context_starts_a_new_codex_thread_with_current_agent` | `crates/session/tests/harness_lifecycle.rs` |
| Server → Node 调度、无原生模型调用、环境值隐藏与续聊 | `server_dispatches_codex_to_node_and_replays_native_messages` | `crates/worker/tests/harness_codex.rs` |
| 内置与自定义 Agent Harness 配置 API | `web_agents` 集成测试 | `crates/web/tests/web_agents.rs` |
| 旧 schema 迁移和无外部状态的历史会话 | `legacy_tables_without_version_row_converge_on_open` 及既有迁移测试 | `crates/store/tests/schema_bootstrap.rs`、`crates/store/tests/store_migrations/` |
| Web 配置、启动与实时 / 刷新折叠结构 | `harness` DOM 测试和 `agentsConfig` 测试 | `crates/web/spa/src/harness/`、`crates/web/spa/src/agentsConfig.dom.test.jsx` |
| 内嵌 SPA 与真实 Server / Node / 二进制 | `scripts/acceptance/harness/codex.js` | 临时目录运行，覆盖提交、展开、刷新、续聊 |
| 无原生凭据、连续恢复、Harness 切换、混合编排、子任务及四类取消 | `codex_orchestrators_without_native_credentials` | `crates/worker/tests/harness_matrix.rs`、`crates/worker/tests/harness/` |
| Plan 仍校验引用、拒绝时不改接收记录 | `rejected_project_commands_never_change_durable_admission` | `crates/worker/src/operations/admission_tests.rs` |
| 交付副本、无效路径、清单大小与符号链接 | `declared_files_are_immutable_and_invalid_paths_fail`、`oversized_or_symlinked_manifest_is_rejected` | `crates/project/src/trace/codex.rs` |
| 旧 Node 不得接收 Harness 请求 | `legacy_node_cannot_join_or_receive_harness_assignments` | `crates/control/src/transport/hub_tests.rs` |
| 真 Codex Project 规划、执行、清单与不可变交付 | `scripts/acceptance/harness/project.js` | 通过最终包 Server / Node 执行 |
| 四件套构建元数据与旧版回滚 | `build_info_matches_platform_without_server_credentials`、`test_upgrade_adds_control_cli_and_rollback_restores_legacy_set` | `crates/ctl/tests/build_info.rs`、`scripts/platform/test_install_bundle.py` |
| 首次模型配置长路径仍可见 | `onboarding_wraps_long_config_path_without_losing_filename` | `crates/tui/src/onboarding.rs` |
| 跨平台子进程取消和终端隔离 | `codex_cancel_reaps_descendants_and_closes_open_tools`、`bash_tool_detaches_controlling_terminal` | `crates/session/tests/harness_codex.rs`、`crates/session/tests/tools_contract.rs` |
| MySQL 并发尝试排斥与终态后领取 | `mysql_project_crud_contract` | `crates/store/tests/sql_project_store.rs` |

## 最终验证与发布

- 干净提交 `9046a052ae0605063c3ff091c42e7d7d45d53137` 已推送 main，并发布到本机 `18081` Server 和唯一节点 `human-os-02`。`/usr/local/bin` 与 `/root/.local/bin` 均安装同一包的四个二进制；实际进程哈希、build-info 和内嵌 SPA 与 manifest 一致。
- Rust：353 个测试目标，**4,929 passed / 0 failed / 5 既有 ignored**；clippy 全 workspace/all-targets 零警告，workspace build 与 release build 通过。SPA 56 文件、473 项通过，发布重新构建并验证 dist 无漂移。平台安装/归档工具 19 项通过，真实四件套与旧三件套升级、回滚、再升级通过。
- CI 修正后的干净提交 `be76fc1086cbf0d928c1d1e03ad5470563fd86df` 再次通过全 workspace 检查：**4,929 passed / 0 failed / 5 既有 ignored**，353 个测试目标，fmt、clippy、配套二进制预构建与 workspace build 均通过，记录为 `post-ci-gates.json` 和 `post-ci-*.log`。
- 后续提交仅包含规范格式、测试和 CI 修正；`source-equivalence.json` 验证运行逻辑与已部署包一致。Mac 路径断言按规范路径比较，取消/steer 测试分离启动等待与五秒取消上限，使用三秒启动延迟和跨平台进程退出检查完成 8 项回归，旧 Bash 用例改用 getsid 验证独立会话并完成 18 项工具契约回归；MySQL 测试先验证旧运行阻止新领取，再结束旧运行后验证原子领取。真实 MySQL 8.4.11 契约及 SQL 可选后端 clippy 通过。最终 [macOS CI](https://github.com/MoSunDay/opencoder/actions/runs/34272427213) 与 [MySQL / fmt / clippy CI](https://github.com/MoSunDay/opencoder/actions/runs/34270311636) 均通过。
- 已安装 CLI 真实 Codex 读取文件与环境后返回正确结果；现网浏览器验证三层折叠、刷新、续聊、环境值隐藏、非法环境零派发与 390px 布局。原生 Agent、DAG、Team、Project、大脑稳定 request_id 与 interrupt 全部通过。
- 自定义 Codex Agent 经只读 NFS 引用版本化 prompt，使用默认 Harness 完成 Project Execute；原生 Plan 与 Codex Execute 混合链路通过。22 字节交付文件 SHA-256 为 `b0c0a4f20e9d48e4558ffe43625317a3622be95eb7db7bb2c5d9f640caa0232b`，修改工作副本后归档不变。纯 Codex Plan/Execute、resume/fork/取消及混合编排另有进程矩阵和真实 Codex 候选验收记录。
- 发布前冻结、interrupt 空闲会话、停写备份，并在新目录完成恢复。实际 definitions/runtime 数据库从 schema 21 迁至 22，迁移进程分别约 12 ms / 20 ms；所有旧行与索引哈希保持，再次打开收敛，旧二进制可读取恢复副本。保留 34 条旧原生会话、38 条原执行索引、36,302 个历史归档文件与 48 个原资源文件，没有删除业务数据或变更鉴权数据。
- 稳定观察：北京时间 **2026-09-09 02:45:43 至 2026-09-09 04:46:13**，**7229.72 秒 / 227 次采样**；Server/Agent 无重启，Node 持续 Ready，固定二进制/入口配置保持，无新增服务错误、超时 Pending 或未收敛 interrupt。

完整发布包、日志、截图、数据库与资源备份、恢复演练、逐字节历史重放及最终记录：`/var/tmp/opencoder-wrap-rollout-20260909-4mztpbel`。主要证据为 `gates-result.json`、`post-ci-gates.json`、`ci-final.json`、`online-verification.json`、`production-observation.json`、`done.json`。

## 本机运行配置与回滚

Node 通过专属 PATH `/usr/local/libexec/opencoder-harness/bin` 使用前台 Codex 入口，该入口复用本机已有 SOCKS5 代理凭据并直接执行 `/usr/local/libexec/codext.real`；不启动 screen，不把凭据写入服务配置或仓库。全局交互式 Codex 入口保持原有行为。本机 CLI 可运行：

```sh
opencoder --wrap codex --cmd "需求" \
  --envs "PATH=/usr/local/libexec/opencoder-harness/bin:$PATH"
```

Server/Node 必须按 Fleet v5 成套升级；旧/新节点混接在注册前拒绝。`config-rollback.json` 保存入口与 systemd drop-in 的不可覆盖原始锚点，并通过中断、matched 重试和显式 restore 演练。数据回滚仅恢复到新目录，按服务身份设置 owner/权限后再切换路径；原目录保留，旧程序不能继续新 Codex 会话。操作见发布目录 `ROLLBACK.md` 和[平台部署](../../../docs/agent-platform.md)。

无关的 `crates/worker/src/dependency/` 没有纳入提交或发布。使用说明见 [Agent Harness](../../harness/index.md)。

## 与远端控制 CLI 合并

保留远端 `c853064e` 的 `opencoder-cli` 控制面客户端和 `crates/cli → crates/local` 更名，以及 `36d2a048` 的 Server 日志模块清理。wrap 参数、headless 入口和 tmux 转交随本地前端迁入 `local`。发布包现在同时包含 `opencoder`、`opencoder-cli`、`opencoder-server`、`opencoder-agent`；四者使用相同完整 build-info，旧三件套可成套升级、回滚。

新增验证：`crates/ctl/tests/build_info.rs::build_info_matches_platform_without_server_credentials`；`crates/ctl/tests/server_local.rs::agents_card_lifecycle_and_active_pointer` 同时校验内置与自定义 Agent、Codex 设置及保留资源引用；`scripts/platform/test_install_bundle.py::test_upgrade_adds_control_cli_and_rollback_restores_legacy_set` 校验控制 CLI 新增与旧版回滚。

实际发布回归使用独占 Cargo target，预先构建配套二进制；临时 Node 数据放在容量充足的 `/var/tmp`。共享 target 曾混入其他提交，默认 `/tmp` 所在盘低于既有 20% 可用容量阈值，两者均不能作为本次有效测试环境；没有降低容量保护。

完整回归还暴露首次模型配置向导在较长配置目录下裁掉文件名的问题；提示段落现按终端宽度换行，`onboarding_wraps_long_config_path_without_losing_filename` 验证路径末尾可见并保持密钥遮罩。
