Commit: 0c6df23d86d0ec9da564868745711c9df0b84259

# 剥离外部业务域，回归引擎与平台仓库

本仓库曾随三条外部业务工作流带入代码、测试、脚本与文档，与引擎面长期混杂。本次按仓库边界一次性剥离：仓库只保留引擎与平台，业务计划与业务资产（计划版本、DAG 定义、Agent how、部署编排）不落本仓库，由 NFS/外部业务仓库维护；被移除资产全部可从 git 历史恢复，精确清单以本提交 diff 为准。

三个批次各自收口：

- 代码审查发布门禁：删除独立审查工具 crate（含审查执行与测试环境起停两个 bin）、`examples/dag/code-review.json` 门禁 DAG 示例、`examples/dag-modules/` 下 wasm 知识索引模块示例、`scripts/platform/` 下 CLI 门禁脚本、`docs/ops/` 审查运维文档、control 启动时的门禁种子 DAG 初始化（`seed_dags.rs` 整文件），以及根 e2e 的门禁用例与 review DAG 套件（`tests/dag_e2e/code_review.rs`、`review_dags/`）。
- PC 问题诊断：删除 core brain 的 PC 诊断计划域与输出契约、诊断计划文档（`docs/` 下）、control 诊断路由与附件图片模块、worker 取证（evidence）逻辑、`scripts/` 下配套诊断脚本，以及 SPA 诊断页面（workbench problem 视图与 DOM 测试）。
- 设备实验室：删除 `crates/dag` 的 devices 与 ui_cases 域、`dag-runtime` `exec/device` 设备执行器与私有客户端传输、`deploy/` 下设备用例与设备管理两个目录，及设备管理服务打包脚本。

同步清理文档与记忆：`features/index.md` 移除代码审查与 PC 诊断入口，`features/brain`、`agents/control`、`agents/dag-runtime`、`agents/brain` 索引删除对应业务描述（门禁种子、PC 计划安装入口、设备执行器、历史数据清理维护脚本），`agents.md` 收缩根 e2e 描述并重申仓库边界；changelog 按条判断——纯业务条目整篇删除，混合条目只去掉业务句段（如知识库只读挂载条目保留通用 `knowledge_root` 描述）。

引擎面保持不变：session/llm/store/shellguard/tui/web/local、node/server/control/worker/agent、DAG 运行时（dag + dag-runtime + dag-wasm 版本池）、版本化 Agent 池、brain 分层调度引擎、todos/project/team 全部保留，根进程级 e2e（operator/dag/todos/team/brain）继续覆盖这些能力。

## 验证

隔离 HOME（`HOME=/tmp/fakehome`）下全量回归，先 `cargo build --workspace --bins`（根 e2e 依赖兄弟二进制）：

- `cargo fmt --all -- --check` 通过；clippy 目标 crate（core/dag/dag-runtime/control/worker/brain/cli/根包）`--all-targets` 无新增告警（既有 `brain/tests/milestone.rs` iter_nth 告警保持基线不动）。
- Rust 全绿：core/dag/dag-runtime、control（lib + e2e 204）、worker（除下述既有项）、brain、store/web/node/agent/tui/local/cli 各集成测试；根 e2e operator 11、dag 10、brain 2、brain_layered 5、team 1、running_mode 2、daemon_smoke 1。
- 既有失败（已在基线 9637c101 worktree 复现，非本迭代引入）：todos_e2e 3 项（agent 二进制 tokio 工作线程栈溢出）、shellguard classify_in_tests 3 项、worker harness_matrix 1 项（进程内栈溢出）。
- 顺带对齐基线上即失败的陈旧断言（均落在本次编辑面内）：brain_e2e/brain_layered_e2e 的 schema 字面量与 fixture 从 6 升至 7（v7 计划形状：节点 `capability_id`/`layer_id`、显式 `layers`/`transitions`、非末层必须有前向转移、评估按 layer_id 单键、`transitions:[]` 不再合法的空转移），responder 改按键 `assessment_layer_id`；`brain/tests/layered.rs`、`ctl/tests/parse_project_brain_agents.rs` 版本字面量同步 7。
- SPA 重建 + `scripts/check-spa-drift.sh` 无漂移；vitest 904 项全绿。
- 业务标识符零命中 grep 门禁：按任务给定业务词清单全仓检索（排除 .git/target/node_modules/dist/Cargo.lock），0 命中。
