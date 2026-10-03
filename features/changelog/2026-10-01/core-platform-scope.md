Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 核心仓库保留通用平台能力

专用业务实现、表单、安装入口、测试及说明移出核心仓库。平台继续提供用户注册的能力与工作流、不可变定义和输入、通用附件、私有任务文件，以及统一的执行和调度接口。核心构建和回归不依赖外部业务工具。

全局复核覆盖配置、依赖、CI、代码注释、模块索引和历史文档，清除已退出实现的引用与具体部署记录。交叉编译说明改为可复用的本地配置与产物检查步骤，嵌入模型说明保留独立 Provider 路由契约；原稿在仓库外保留。

技能契约测试清除退役外部协议和专用工具的名称，改用自定义用户资源与准确的内置技能、资源集合验证种子写入，并拆分为四个模块，原有 28 个用例保留。已退出业务的配置注释和历史说明原稿保存在仓库外。SPA 产物检查只复制包内资源，不再带入示例目录；身份加载与资源选择的 DOM 测试按当前界面操作，保留导航、保存、固定版本和错误提示断言。

节点锁在释放时显式解锁，停机等待任务析构结束，避免继承的描述符阻止重启。浏览器验收按现有画布验证自动保存路径，并通过“查看详情”检查运行与执行记录。

资源追加测试复用资源模块已有的全局目录锁，避免与引用扫描同时改写测试目录；追加内容、保留同级文件和版本递增的断言保持完整。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 注册工作流的定义和输入原样固定，缺失目标拒绝受理 | `registered_workflow_definitions_and_inputs_are_pinned_without_rewriting` | `crates/control/tests/e2e/executions_submit.rs` |
| 首次安装的技能集合准确、无缺失或多余条目 | `seed_in_writes_all_packs_on_fresh_dir` | `crates/core/tests/skill_contract/seeding.rs` |
| 内置文件更新前备份，自定义用户资源不改写、不备份 | `seed_builtin_skills_backs_up_then_overwrites_user_edits` | `crates/core/tests/skill_contract/seeding.rs` |
| 首次安装只有完整、准确的内置规划资源集合 | `seeded_task_plan_skill_requires_launch_closure_contract` | `crates/core/tests/skill_contract/planning.rs` |
| 身份加载后导航、节点选择与保存状态保持正确 | `renders the brand and the node-category menu on the default page`、`lands the fleet tab preselect on the sidebar node select` | `crates/web/spa/src/appNavigation.dom.test.jsx`、`crates/web/spa/src/sidebar.dom.test.jsx` |
| 实际资源和版本选择保存，结构编辑不丢并发配置 | `选中节点后属性面板编辑命令并保存`、`结构编辑保留顶层 max_concurrency（加步骤后保存不丢并发配置）` | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 通用图片附件往返、摘要及非法上传拒绝 | `image_attachments_roundtrip_and_reject_invalid_uploads` | `crates/control/tests/e2e/layered_api/attachments.rs` |
| 共用结构化输出解析器拒绝未闭合 JSON | `broken_fence_falls_back_to_tail_bare_json` | `crates/dag-runtime/src/exec/agent.rs` |
| 描述符复制与子进程继承不阻止节点重启 | `dropping_worker_releases_node_lock_despite_a_duplicated_descriptor`、`dropping_worker_releases_node_lock_while_a_forked_child_keeps_its_descriptor` | `crates/worker/src/state/tests.rs` |
| 停机等待执行及后台任务析构 | `shutdown_waits_for_execution_task_destructors`、`shutdown_waits_for_background_task_destructors` | `crates/worker/src/state/tests.rs` |
| 资源追加与引用扫描共用测试目录锁，避免并发覆盖 | `append_preserves_siblings_and_grows_versions`、`scan_tools_lists_children_excluding_meta` | `crates/agents/src/resources/how_append.rs`、`crates/agents/src/references.rs` |
| 真实画布保存、两轮调度、执行记录与窄屏详情 | 浏览器运行验收 | `scripts/acceptance/brain/runtime.js` |
| 容器任务连续运行、运行时交接、回滚与连续提交 | 16 类隔离发布场景，58 次提交无失败 | `scripts/acceptance/smooth_release/main.py` |

- 本轮最终隔离源码全量回归：`cargo test --workspace` → 441 个套件，5634 passed / 0 failed / 7 ignored；未新增跳过项。默认跳过的真实 NFS、容器和浏览器用例此前在旧平台版本另行全部运行通过，其版本与日志分别保留。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；共享测试目录锁修复后重新验证通过。
- 构建：`cargo build --workspace --bins --examples` → 通过；测试前后夹具二进制摘要一致。
- SPA：127 个测试文件、949 项通过；构建与产物一致性检查通过。技能契约 28 项原有用例完整保留，准确技能集合的断言也已包含在最终全量回归中。
- 仓库外业务版本：42 处接入保留，2 处补丁上下文冲突已适配新增通用模块；全目标编译检查、32 项 Rust 测试、7 项前端测试及 3 项组装校验通过。
- 此前发布和维护脚本：59 项、29 项通过；工作区与预检校验各 6 项通过。旧平台版本的隔离发布演练覆盖 16 类场景和 58 次提交，保留当时源码、二进制与回执；未发布生产。
- 全量回归使用独立临时目录及 tmpfs；超时、断言和容器/NFS 约束保持原值。共享资源目录测试改为共用既有锁，保持原有并发测试参数。
- 原始输出位于 `/data00/opencoder-safety/20261001-103221/`：`confirmation-frozen-workspace-tests.log`、`confirmation-frozen-clippy-after-lock-fix.log`、`confirmation-frozen-build-after-lock-fix.log`、`confirmation-latest-spa-tests.log`。固定的 1745 份 Rust 源码及清单文件摘要见 `confirmation-frozen-source.json`；源码汇总 SHA256 为 `385c32d592c38bba6212221f7d3f88ac8f854d59365a9dbc305a45b1f81865c0`。
- 全局扫描及迁移摘要、外部业务验证、已解决的失败和并行开发差异见 `/data00/opencoder-tools/confirmation-report.json`。共享工作区仍在同时迭代其他通用平台功能；上述 Rust 全量结果仅对应固定源码清单，后续改动的验证范围单独记录。

[逻辑地图](../../../agents.md) · [Control](../../../agents/control/index.md) · [Worker](../../../agents/worker/index.md) · [大脑工作台](../../brain/index.md)
