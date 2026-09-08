Commit: (working-tree, 基于 65c9d891ae905e7925277d29a87cd8e7957e8dad)

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

## 验证

- SPA：56 个测试文件，**473 passed / 0 failed**；构建及 dist 无漂移。输出 `/var/tmp/opencoder-wrap-closure-spa-tests-final.log`、`/var/tmp/opencoder-wrap-closure-spa-build-final.log`。
- Rust 最终完整回归：`cargo test --workspace --no-fail-fast -- --test-threads=4` → **4,840 passed / 0 failed / 5 ignored**（既有手动测试，未新增 ignore）；341 段 `test result` 汇总，输出 `/var/tmp/opencoder-wrap-closure-delivery-tests.log`。本轮闭环基线为 4,836，通过数增加 4。
- `cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo build --workspace` 通过，输出 `/var/tmp/opencoder-wrap-closure-delivery-clippy.log`、`/var/tmp/opencoder-wrap-closure-delivery-build.log`。
- Codex 0.153.2：CLI 新建、读取文件/环境、resume、fork 和取消已验证。最终候选包 CLI 记录在 `/var/tmp/opencoder-wrap-delivery.94_bwc03/final-cli/`；早期 resume/fork 与脱离进程组回收记录保留在 `/var/tmp/opencoder-wrap-real-resume.stdout`、`/var/tmp/opencoder-wrap-real-fork.stdout`、`/var/tmp/oc-wrap-cleanup-9564xq65/`。
- 最终候选包浏览器：夹具（含工具失败后恢复）与真实 Codex 均 PASS。真实记录 `/var/tmp/opencoder-wrap-closure-final-real-browser.log`，截图及 Project 验收 `/var/tmp/opencoder-wrap-browser-aKt7Ah/`。Project 交付精确为 21 字节，验证工作副本修改后归档不变；Plan 输入不含本次执行的清单路径。
- 平台安装/归档工具 18 项测试通过。旧版本 → 候选 → 回滚 → 候选的实际三二进制元数据验证通过；新 Server/旧 Node、旧 Server/新 Node 均拒绝，注册数和执行数为 0。
- 隔离迁移演练：旧二进制创建 schema 21，填充 10,000 会话 + 10,000 消息后升级为 22，计数与完整性保持，旧会话运行态为 NULL；重复打开和旧二进制读取历史原生会话通过。数据库约 2.56 MB，迁移进程 0.03 秒。此为合成数据，不代表生产窗口测量；未修改业务数据库或鉴权数据。

Rust 回归使用独立 loopback 网络命名空间、系统盘 `TMPDIR=/var/tmp/oc-wrap-tests`，仅移除测试进程代理变量。中间版本出现过定义校验失败、构建替换导致旧测试产物缺失，以及浏览器定位器不识别错误标签；最终结果以上述固定源码的完整回归和真实验收为准。

## 候选交付与兼容性

最终包：`/var/tmp/opencoder-wrap-delivery.94_bwc03/release`，独立干净源码提交 `9bb6e2cc1c74c5ebdc5ae2dc2a5f206e70a93ea5`。三个二进制的提交号、协议 v5、SPA 摘要及校验和一致；运行时代码与当前工作区逐项一致。最后的浏览器定位器增强仅影响验收脚本，不改变包内代码。

当前安装历史 ff43bfa9 / 83ff58e8 的修改已包含于工作区基线（patch-equivalent）；候选保留既有功能。无关的 `crates/worker/src/dependency/` 未纳入候选。主工作区分支与索引未提交，现有安装和运行服务未切换。

上线使用同一包同步升级 Server/Node，执行节点 PATH 指向能输出 exec JSONL 的前台 Codex 入口。本机前台入口为 `/usr/local/libexec/codext.real`，隔离前缀为 `/var/tmp/opencoder-wrap-delivery.94_bwc03/codex-bin`。升级前停写并备份数据；回滚成套二进制，旧程序不能继续新的 Codex 会话。具体产物、安装/回滚与数据验证见交付目录中的记录。

使用说明见 [Agent Harness](../../harness/index.md)。

## 与远端控制 CLI 合并

保留远端 `c853064e` 的 `opencoder-cli` 控制面客户端和 `crates/cli → crates/local` 更名，以及 `36d2a048` 的 Server 日志模块清理。wrap 参数、headless 入口和 tmux 转交随本地前端迁入 `local`。发布包现在同时包含 `opencoder`、`opencoder-cli`、`opencoder-server`、`opencoder-agent`；四者使用相同完整 build-info，旧三件套可成套升级、回滚。

新增验证：`crates/ctl/tests/build_info.rs::build_info_matches_platform_without_server_credentials`；`crates/ctl/tests/server_local.rs::agents_card_lifecycle_and_active_pointer` 同时校验内置与自定义 Agent、Codex 设置及保留资源引用；`scripts/platform/test_install_bundle.py::test_upgrade_adds_control_cli_and_rollback_restores_legacy_set` 校验控制 CLI 新增与旧版回滚。

实际发布回归使用独占 Cargo target，预先构建配套二进制；临时 Node 数据放在容量充足的 `/var/tmp`。共享 target 曾混入其他提交，默认 `/tmp` 所在盘低于既有 20% 可用容量阈值，两者均不能作为本次有效测试环境；没有降低容量保护。
