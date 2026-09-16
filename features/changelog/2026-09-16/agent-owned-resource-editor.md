Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# Agent 按身份直接编辑资源

Agent 配置由共享池引用选择改为按 Agent 名称查看正文与编辑资源。新建仅填写名称和执行方式，创建后打开抽屉，首次保存自动创建独立资源。Prompt、Skills、Tools 和 Memory 全部可编辑；内置 Agent 展示实际定义和工具限制，资源只读。

- 新增 Web / Control 共用的 `GET/PUT /api/agents/:name/resources/:cat` 与 `POST .../restore`。读取固定版本及历史、递归文件清单；保存携带资源、版本、revision 基线，冲突返回 409。
- 共享资源首次编辑/恢复复制完整历史并绑定 `owner_agent`，仅切换当前 Agent；旧资源池 API 禁止修改专属资源，其他 Agent 禁止绑定。`tools_scope=all` 当前 Agent 工具优先并保留共享工具，排除其他 Agent 专属工具。
- 文件变更纯函数合并，保留未修改文件、二进制字节及权限；统一资源根跨进程写锁，目录完整落盘后切换生效指针。拒绝不安全路径、符号链接、无效结构、超限资源和过期基线。节点快照忽略未发布 staging，已接受任务继续使用原快照。
- 文件树、文本编辑器、预览、上传/替换、下载、目录与文件重命名/移除；草稿跨页签保留，关闭/刷新确认，读取失败禁写，保存失败保留草稿。旧共享池 PUT 支持省略 body 名称，并拒绝与 URL 名称不一致的请求。
- 不新增数据库表或环境变量；交付代码与构建产物，无部署操作。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 四类资源隔离、完整历史、附件、权限、工具范围 | `shared_resources_fork_all_categories_with_history_bytes_and_modes` | `crates/web/tests/agent_resources/isolation.rs` |
| 并发 409、历史恢复与版本递增 | `restores_create_versions_and_stale_or_parallel_saves_conflict` | 同上 |
| 内置定义和只读 | `builtins_show_real_definitions_and_resource_writes_are_forbidden` | 同上 |
| 路径、结构、重复、超限拒绝 | `rejects_bad_paths_shapes_duplicates_and_oversize_without_publishing` | `crates/web/tests/agent_resources/errors.rs` |
| 共享历史缺失时停止复制并保留原引用 | `incomplete_shared_history_aborts_fork_without_changing_reference` | `crates/web/tests/agent_resources/errors.rs` |
| 缺省历史的旧元数据兼容 | `legacy_current_only_metadata_can_be_read_and_forked` | `crates/web/tests/agent_resources/isolation.rs` |
| 资源版本变更后 Meta 内容同步 | `owned_version_edits_refresh_visible_reference_contents` | `crates/web/tests/agent_resources/isolation.rs` |
| 读取/写入失败、符号链接与旧 API 身份 | `read_errors_symlinks_and_failed_writes_never_become_empty_overwrites`、`legacy_put_uses_url_identity_and_checks_reference_baselines` | 同上 |
| 真实 HTTP 保存 → Prompt、技能、可执行工具及 Memory 注入 | `http_saved_resources_feed_prompt_skills_executable_tools_and_memory` | `crates/web/tests/agent_resources/runtime.rs` |
| Control 配置根与相同资源接口 | `custom_agent_publication_uses_configured_root_without_cross_server_leaks` | `crates/control/tests/resource_root.rs` |
| 已接受快照与发布暂存隔离 | `private_resources_are_pinned_but_unpublished_staging_is_excluded` | `crates/worker/tests/resource_snapshot.rs` |
| 文件合并保留字节/权限、路径拒绝 | `merge_preserves_bytes_modes_and_rejects_ambiguous_changes` | `crates/agents/src/resources/model.rs` |
| 直接查看、草稿、首次保存、恢复、上传、重命名、错误、只读 | `AgentDetail direct resources` | `crates/web/spa/src/agentDetail.dom.test.jsx` |
| 简化新建、抽屉关闭确认 | `AgentsPanel` | `crates/web/spa/src/agentsConfig.dom.test.jsx` |
| 目录重命名与二进制字节、路径校验 | `resourceModel` | `crates/web/spa/src/agents/resourceModel.test.js` |

- 相关前端变更前基线：24 passed。
- SPA 全量：`npm --prefix crates/web/spa test` → 104 files / 752 passed。
- 补充前端校验后：AgentDetail 与资源纯函数 14 passed。
- SPA 构建：`npm --prefix crates/web/spa run build` 通过；沿用单 bundle 构建，保留既有体积提示。
- Chromium + 真实 CodeMirror 冒烟：四类保存、跨页签草稿、二进制替换与权限、重开回读通过。HTTP 边界独立由真实监听端口的 Rust 集成测试验证。
- 真实 HTTP 专项：`cargo test -p opencoder-web --test agent_resources` → 10 passed / 0 failed（isolation 5、errors 4、runtime 1，真实监听端口）。
- 关联 Rust 套件：control `resource_root` 7 passed、worker `resource_snapshot` 1 passed、`opencoder-agents` 31 passed（1 项既有 ignored）、control e2e `compat_nodes`/`sessions_compat_extra`/`sessions_relay` 全绿、dag-runtime `run_loop` 8 passed、cli `server_local` 全绿。
- rules/02 全量门（独立 `CARGO_TARGET_DIR=/data00/rust-build/cargo/opencoder-release-20260916`、`CARGO_PROFILE_DEV_DEBUG=0`）：`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo test --workspace -- --test-threads=4` → 403 个测试二进制全 ok、5264 passed / 0 failed（既有 6 项 $HOME 竞态本轮未复现）；`cargo build --workspace` 零错误。日志：`/var/tmp/opencoder-release-20260916/06-workspace-test.log`、`05-full-gate.log`。
- SPA 门：104 files / 752 passed，`vite build` 通过，`scripts/check-spa-drift.sh` 无漂移。
- 发布工具链自检：signal_tests 12、rolling_tests 19、platform 19、smooth_release 4 全部 OK。

## 已知行为（本轮不修复）

- 删卡（`crates/agents/src/write.rs::delete_agent`）只移除 `<agent>/` 卡片目录，不回收已绑定的 `agent-<uuid>` 专属资源目录。孤儿目录不可被其他 Agent 绑定、不进入任何 Agent 的 `tools_scope`，也不被 NFS 快照引用，仅占磁盘；GC 另立后续任务。
