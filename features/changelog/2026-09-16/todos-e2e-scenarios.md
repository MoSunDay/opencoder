Commit: 30108c8be3b60a11d2a6b41b0b9b482529678209

# todos e2e 场景矩阵补全（E21 契约 / E22 运行时 / E23 web 面）

## 背景

E19b/E19c（`todos_scenarios.py`）只覆盖了 CLI 主流程，三块契约仍无 e2e 固化：① validate/绑定/观测面这类**无 key 即可判定**的本地契约；② 运行时**输出文档与事件目录**、确定性失败闭环、本地 Ctrl-C 与 `--debug` 投影刷新、目录格式 + env 透传；③ serve 端**模板/环境生命周期**与无节点 run 拒绝。本次不改生产代码，只把契约固化为三个新套件并接入 `e2e_glm.py`。

## 变更

- 新增 `scripts/e2e/todos_contract_scenarios.py`（E21，399 行，无 key）：
  - E21a 三种输入形态合法 + store-free validate（XDG_DATA_HOME 覆写，全程无 opencoder.db 落盘）；
  - E21b 9 类畸形 spec 的 `path:line:col: message` 中文诊断归因（含 `t/1` 仅 manifest 列出、agent 冲突、cycle）；
  - E21c 通过 `agent.share_dir` 种子环境验证 env.json 绑定四态（合法 / 环境缺失 / 工具缺失 / env_vars 键非法）；
  - E21d 空库 list、未知 id 五命令 exit 1、不可达端点 run → `suspended` 记录 + `terminal_reason`。
- 新增 `scripts/e2e/todos_runtime_scenarios.py`（E22，399 行，需真模型 key，本环境无 key 故为待验证写入）：
  - E22a 成功主流程输出契约（stdout 纯 JSON 状态文档、stderr `workflow_id=`、事件目录全量、`{seq,workflow_id,kind,payload,ts}` 形状、`--after` 游标连续后缀、`show` 单行 banner、pretty run == compact resume）；
  - E22b 确定性失败（max_attempts=1 + 永不通过的 required_tool_calls 门）：exit 1 + terminal_reason、`todo_failed`/`workflow_failed` 事件、无 attempt-002 会话、resume 幂等（0 新事件）、终态 interrupt 拒绝；
  - E22c 本地 SIGINT → rc130 + "local interrupt requested" + 投影刷新（workflow_interrupted / suspended index）→ `resume --debug` 补 workflow_resumed 且 index 同步；
  - E22d 目录格式 spec + env.json 绑定 → env_vars 透传到 bash 子进程（marker 文件落值）。
- 新增 `scripts/e2e/todos_web_scenarios.py`（E23，333 行，29 项全绿）：
  - E23a 模板生命周期 HARD 契约：create/revision、重复 create 409、validate-files 400 + diagnostics 四字段、new-version 需 expected（缺省 409 / 过期 409）、版本只读（PUT context.json / env.json → 409）、DELETE current 409、DELETE 非 current ok、保留期钳到 10 版且 `pruned` 命名被裁剪目录（其 /files → 404）；
  - E23b 环境生命周期：create `{ok,name}`、重复 409、list 含 env_vars 回显；
  - E23c run 面按**部署语义**（compat 节点调度）：未知模板 400、无在线节点 503 "no ready online node"、未知 workflow id 在 get/interrupt/resume/events 均 404、workflows 列表形状；若真有节点在线则对 run→记录→SSE→终态引用做 best-effort 驱动。
- `scripts/e2e/lib.py` 新增 `run_split`（stdout/stderr 分离捕获）与 `json_or_none`。
- `scripts/e2e_glm.py` 注册三个套件：contract 随 cli/全量模式（无 key 依赖）、runtime 随 cli 模式（需 key）、web 套件随 web 模式。
- 文档：`agents/todos/index.md` 新增「e2e 套件」清单、`features/todos/index.md` 反链、README 开发与测试段落。

## 关键发现（记录）

- serve 端 `/api/todo/templates/:name/:version/run` 在 `opencoder-server` 上被 compat 路由接管为**节点调度**（`crates/control/src/api/compat/mod.rs:35`），与 `crates/web` 的本地 runtime 语义不同；无在线节点时 503 在建执行记录**之前**返回，不落任何记录。
- share 根解析优先级：`OPENCODER_SHARE_DIR` > `agent.share_dir` > `~/.opencoder/share`（全局）。web 套件必须用 `agent.share_dir` 隔离，否则会污染全局 share 且不可重跑。
- 版本保存（new-version/DELETE）都会重写 `todo.json`，`revision` 随之变化——前端/测试必须在每次变更后重读 revision。

## Impact Surface

- 新增 `scripts/e2e/todos_contract_scenarios.py`（399 行）、`todos_runtime_scenarios.py`（399 行）、`todos_web_scenarios.py`（333 行）
- 修改 `scripts/e2e/lib.py`、`scripts/e2e_glm.py`、`agents/todos/index.md`、`features/todos/index.md`、`README.md`
- 生产代码零改动

## 测试覆盖

| 契约 | 场景 | 文件 |
| --- | --- | --- |
| 三输入形态合法 + store-free validate | E21a | `todos_contract_scenarios.py` |
| 9 类畸形 spec 诊断归因（path:line:col 中文） | E21b | 同上 |
| env.json 绑定四态（share_dir 种子） | E21c | 同上 |
| list/未知 id/不可达端点 → suspended 记录 | E21d | 同上 |
| 成功主流程输出文档 + 事件目录 + 游标 | E22a（需 key） | `todos_runtime_scenarios.py` |
| 确定性失败闭环 + resume 幂等 + 终态 interrupt 拒绝 | E22b（需 key） | 同上 |
| 本地 Ctrl-C rc130 + `--debug` 投影刷新 | E22c（需 key） | 同上 |
| 目录格式 + env_vars 透传 | E22d（需 key） | 同上 |
| 模板生命周期 18 项（含保留期裁剪） | E23a | `todos_web_scenarios.py` |
| 环境生命周期 3 项 | E23b | 同上 |
| 无节点 run 拒绝 + 未知 id 404 面 | E23c | 同上 |

## Validation

- `python3 scripts/e2e/todos_contract_scenarios.py` → 44 passed, 0 failed（跑两遍）
- `python3 scripts/e2e/todos_web_scenarios.py` → 29 passed, 0 failed（跑两遍，serve 每次 `agent.share_dir` 隔离）
- `python3 scripts/e2e/todos_runtime_scenarios.py` → 本机无 ZHIPU_API_KEY/auth.json，干净 SKIP（待有 key 后随 `scripts/e2e-glm.sh` 执行）
- `python3 -m py_compile` 三套件 + `e2e_glm.py` 通过；行数均 < 400

## Related Docs

- [agents/todos](../../../agents/todos/index.md)
