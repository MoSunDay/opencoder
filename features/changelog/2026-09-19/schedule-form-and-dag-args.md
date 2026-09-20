Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# 调度页提示移除 + 新建表单简化 + params 按类型分流（agent→how.md / dag→命令行传参）

## 变更

- 调度页移除「定时任务存于控制面数据库（schedules.json 仅作首次导入种子），每 N 秒扫描一次」整条提示及 `scanSecs` 状态（`spa/src/schedule/panel.jsx`）。
- 新建表单简化（`spa/src/schedule/editor.jsx`）：隐藏 ID 与时区字段——ID 缺省由后端生成 `schedule-<ULID>`，时区固定 `+08:00`（需其它时区创建后编辑修改）；编辑模式保留两字段（ID 主键、时区可改）。kind 下拉收敛为后端 `ScheduleKind` 的五种（agent/team/todos/dag/brain），移除保存必 400 的 project/operator。
- params 从「JSON TextArea + dag 禁用」改为按 kind 分流的普通文本输入：agent/team/todos → `params.prompt`（agent 触发时作为首轮消息提交，成功后经 `declared_how_append` 回落机制追加进 agent 的 how.md）、dag → `params.args`、brain → `params.objective`（必填）。编辑保存按当前键合并 `initial.params` 其余键（brain 的 inputs/mode/plan 不丢），agent 编辑清掉 `how_append` 旧键避免双源。
- 后端 dag 传参通路（新）：`ScheduleJob::validate` 不再拒绝 dag params，改为 `params.args` 存在时必须是字符串（`config/schedule/validate.rs`）；worker 把 input 改写冻结 spec 的逻辑提取为纯函数 `apply_input`——`prompt` 追加 Agent 步执行要求（原状），新增 `args`（非空字符串）追加到每个 Wasm 步 command（每次 run/resume 基于冻结定义重新 decode，幂等不重复追加）。
- `docs/agent-platform.md` 补 `params` 按 kind 消费键与 dag `args` 追加语义。

## 验证

- unit（opencoder-core）：`dag_params_args_are_accepted_but_must_be_a_string`（dag+args 通过、非字符串报错）。
- unit（opencoder-worker）：`apply_input` 四用例（args 追加 wasm command / prompt 追加 agent 步 / 空值 no-op / 双键共存）。
- e2e（opencoder-control）：`dag_schedule_with_args_creates_and_fires`（create 200 + ledger fired + 确定性 `dag-` 执行 id）；全套 190 passed。
- e2e（根包 dag_e2e）：新增 `input_args::dispatch_input_args_append_to_the_wasm_command_line`（argv-echo wat 模块直证 `input.args` 追加到 wasm 命令行后的 token 序列）；全套 15 passed。
- SPA Vitest：`schedule/panel.dom.test.jsx` 11 用例（新建无 ID/时区、params→prompt 映射、dag→args、编辑合并、提示移除）；全套 114 files / 843 tests passed。
- `npm run build` + `check-spa-drift.sh`：完成，`crates/web/spa/dist/static/app.js` 已更新，无漂移。

## 存量修复（非本需求引入）

- `tests/operator_e2e/agent_session.rs`：981a285f「separate operator and agent dialog lanes」把 `GET /api/sessions` 改为默认 operator 泳道（`?kind=agent` 才列 agent 会话），作者更新了 control 自有 e2e 但漏了根套件，导致 HEAD 上 O5 存量失败（纯净 HEAD 复现，与本需求及并行改动无关，已用一次性副本归属验证）。对齐断言：agent 会话出现在 `?kind=agent` 泳道且不漏入 operator 泳道。`agents.md` O5 描述同步更新。

## 全量回归

- `cargo test --workspace`：426 个测试套件全部 ok（含 operator_e2e O1–O5、dag_e2e D 系列 + agent_runc 真容器、todos/team/brain e2e、control e2e 190）。
- `cargo clippy -p opencoder-core -p opencoder-worker -p opencoder-control -p opencoder --all-targets -- -D warnings`：通过（传递覆盖全部 workspace crate）。
