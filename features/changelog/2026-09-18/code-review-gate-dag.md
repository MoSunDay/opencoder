# M2：code-review 发布门禁 DAG 全链路

## 背景

发布需要一个可机读、可重放的变更审查门禁：以知识库 git 仓库为审查对象，
对 base..head 变更做范围圈定 → 对外 API 行为面/回归护网双锚定 → 双路审查 →
逐条重核 → 出口裁决（P0/P1 实锤即 blocked）→ viking 工单。M2 落地六个部分：
wasm 索引步骤、9 步 DAG 定义、compat input 透传、ctl `--input`、门禁脚本、
进程级 e2e。

## 变更

### 1. kb-index wasm 模块（`examples/dag-modules/kb-index/`）
- 独立 crate（根 `Cargo.toml` exclude），WASI 步骤模块，发布到 wasm 版本池后由
  `command: "kb-index.wasm"` 引用。
- 产物契约：`output.json`（entries/knowledge_root/manifest/truncated/只读探针
  结果）+ `manifest.json`（知识库文件清单，路径相对、字典序，跳过 symlink）。
- fail-closed：`OPENCODER_KNOWLEDGE_DIR` 缺失或非目录 → exit 1（步骤 error）；
  对知识库做一次写探针并只记录结果（失败为预期，只读挂载语义）。

### 2. 9 步 DAG 定义（`examples/dag/code-review.json`）
- 拓扑：kb-index(wasm, runc) → change-scope → api-anchor/harness-anchor（并行）
  → api-review/harness-review → confirm → summary → viking-ticket。
- 每个 agent 步 prompt 首词独占（圈定变更范围/锚定对外 API 行为面/…/创建工单），
  既是 LLM 指令也是 stub 路由键；末尾带 ```json 围栏输出契约。
- summary 裁决规则硬编码进 prompt：任一 P0/P1 实锤 → `blocked`；仅 P2 →
  `pass_with_tickets`；无实锤 → `pass`。

### 3. compat dispatch input 透传（`crates/control/src/api/compat/workflows.rs`）
- `dispatch()` 的 execution input 由「恒为 `{}`」改为取 body 顶层 `input`
  （null/缺失退化为 `{}`）。worker 侧既有行为把 `input.prompt` 以
  「\n执行要求：{prompt}」注入每个 agent 步 prompt 并落
  `<node-data>/dag/<run>/input.json`，本变更让该链路对 compat dispatch 可用。

### 4. ctl `--input`（`crates/ctl/src/cmd/dag.rs`）
- `dag dispatch` 新增 `--input <JSON>`：merge 进 dispatch body 的 `input` 键，
  同键时优先 `--input`；`--json` 顶层非对象时 `input` 退化为 `{}`。

### 5. 门禁脚本（`scripts/platform/code_review_gate.sh`）
- 流程：依赖检查 → kb-index 缺失时 wasm32-wasip1 release 构建并发布到池 →
  `PUT defs`（支持 `--defs-file` 覆盖）→ compat dispatch（带
  `input.prompt=base=… head=… 变更审查请求（发布门禁）`，大 body 走临时文件
  `@file` 避免 ARG_MAX）→ 轮询 run 终态 → 读 summary 步 `verdict`。
- 退出码：verdict=pass → 0；pass_with_tickets/blocked → 1；run error → 1（打印
  progress 逐步 error）；超时 → 1。run id 形如 `dag-cr-gate-<head12>-<unix>`
  （控制面要求 `dag-` 前缀）。

### 6. e2e（`tests/dag_e2e/code_review.rs`，模块声明由主线程统一添加）
- `rig()`：真 fleet + 知识库目录 + `dag.knowledge_root`/`wasm_dir` 全量配置 +
  kb-index wat 替身（相对 preopen fd 3 写 output.json/manifest.json + stdout 摘要，
  与真实模块产物契约一致）+ 9 步 spec（kb-index 走默认 in_process 沙箱，摆脱
  runc rootfs 依赖）。
- `gate_passes_a_clean_change`：无实锤 → run done、progress 9/9、summary
  verdict=pass、viking-ticket not_required；断言 input.json == dispatch input、
  agent prompt 带「执行要求：」后缀与「知识库（只读）」提示、kb-index 双产物、
  全步 meta.json outcome=done、`/steps/summary` 的 `output` 字段。
- `gate_blocks_on_confirmed_p1_findings`：双 P1 实锤 → confirm 确认 2 条、
  verdict=blocked(p1=2)、工单步建单 2 张。

## 测试覆盖

- `tests/dag_e2e/code_review.rs`：2 用例（见上），随 dag_e2e 套件 9/9 绿。
- `crates/ctl/tests/parse_dag_teams.rs`：`dag_dispatch_input_lands_in_the_body_input_key`
  （--input 优先、--json 退化），`--lib` 24 绿。
- `crates/control/tests/e2e/dag_dispatch_extra.rs`：
  `dispatch_passes_input_through_to_the_assignment`（透传至 assignment.request.input）。
- `bash -n` 门禁脚本；python3 结构校验 9 步 spec；kb-index host check 与
  wasm32-wasip1 release 构建均过；真 fleet 冒烟（server+agent+脚本同进程组）：
  发布 201 → defs 200 → dispatch 202 → kb-index done（entries/manifest/只读探针
  全对）→ 死 LLM 端口下 change-scope error → 门禁 exit 1。
