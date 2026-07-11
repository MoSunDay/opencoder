Commit: (working-tree)

# e2e 深度化（业务契约断言）+ `session show --json` 观测面 + agents/cli 记忆补齐

## 背景
原 `scripts/e2e-glm.sh` 只验证**表面标记**：grep 关键字、文件存在、id 不等。核心业务契约从未被 e2e 真正断言——
- `--fork` 只查新 id ≠ 原 id，**不验证是否真复制了消息、原 session 是否真未变**。
- bundle 导入导出只查文件存在 + "imported" 文本，**不验证导入会话与原会话内容一致**。
- `--continue` 只查 scoreboard 关键字，**不验证恢复的会话真加载了首轮上下文**。
- 压缩只 grep 标记，**不验证摘要引用了实际工作**。
- subagent 只 grep 派遣标记，**不验证完成 / DB 跟踪 / 父答案引用**。
- plan 只读、web 两段式 delivery 完全无 e2e。

根因：headless CLI 缺乏机器可读的深度观测面——`session show` 只打印 `m.text()`（仅 Text 块），工具调用 / 结果 / 消息计数 / subagent 记录均不可观测。

## 变更

### A — CLI 观测面：`session show <id> --json`（`crates/cli/`）
- `lib.rs`：`SessionSub::Show` 增 `--json` flag。
- `session_cmd.rs`：`build_session_json(store, id)` 返回 `{meta(含 compaction summary), messages(全 ContentBlock: Text/Reasoning/ToolUse/ToolResult), subagent_tasks(status/result/ok)}`；`show_session_json` 打印之。所有类型已 `Serialize`，Store 已有 `get_session`/`load_messages`/`list_subagent_tasks`。
- 单元测试：`build_session_json_emits_meta_messages_and_subagent_tasks`（验证 ToolUse 块未被过滤成 text）、`build_session_json_errors_on_missing_session`。
- 解锁 e2e 深度断言且**解耦存储内部**（不依赖 sqlite 直查 / `DefaultHasher` 路径）。

### B — e2e 重写为 Python 深度契约套件（`scripts/e2e/`）
- `scripts/e2e/{lib,cli_scenarios,web_scenarios}.py` + `scripts/e2e_glm.py`（入口）+ `e2e-glm.sh`（薄 wrapper）。stdlib only，无第三方依赖。每文件 ≤400 行（规则 03）。
- 每场景断言**业务契约**，区分 hard（确定性存储契约）/ soft（模型协作依赖，记 skip 不 fail）：

| 场景 | 深度断言（相对原 bash 的「表面」断言） | hard/soft |
|------|----------------------------------------|-----------|
| E1/E6 | 日志含 `▸` 工具标记（**工具真被调用**，非纯文本打印）+ 编译 + >50 行 | soft+hard |
| E2 | `show_json` 历史**含 turn-1 内容**（上下文真加载，非新 session）+ resume 追加 turn | hard |
| E5 | fork 后**原 session 角色序列 + 计数不变**；fork 历史前缀 == 原；fork 发散 | hard |
| E3 | 压缩标记 + **摘要引用实际工作**（calc/函数，非通用模板） | soft |
| E3b | 压缩后 `--continue` 产生的 test_calc **引用 calc 的函数**（上下文存活） | soft |
| E4 | subagent 派遣 + **DB `subagent_tasks` 行持久化 + 到达终态** + 父答案引用调查 | soft+hard |
| E8 | export A→import B→**消息计数相等 + 文本内容相同 + 角色序列相同**（完整性） | hard |
| E7/E9 | config→models 显示路径 | hard |
| E10（新） | `--agent plan` + 写文件 prompt → **文件不存在**（plan 只读契约） | hard |
| E11（新） | serve 后台 → POST A(steer)+POST B(queue) → 轮询 `/messages` 至 **2 个 user 消息**（两段都投递）+ app.py 被 A 创建并被 B 扩展 | hard+soft |

- 鲁棒性：live run 未产出 session 标记（运行错误，如 transient 模型/网络失败）时，**soft-skip** 下游契约检查，而非误报契约失败。E11 的「投递」信号用 **user 消息计数**（每个被 drain 处理的 prompt = 1 条 user 消息；steer 即时、queue idle 消费），而非 assistant 消息计数（一个含工具的 turn 会产多条 assistant 消息，会误判）。

### C — 记忆补齐
- 新增 `agents/cli/index.md`（`agents.md` 此前引用为 bare bullet 但无文件）。文档化子命令、全局 flag、headless 事件 marker 集（e2e 日志断言来源）、`build_session_json`、`data_dir_for` 的 `DefaultHasher` 已知隐患。
- `features/index.md` e2e 描述改为深度契约版（12 项 + hard/soft 策略）。

### D — 修复：`serve` 子命令此前忽略全局 `--workdir` flag（e2e 暴露）
- E11 启动 `opencoder --workdir <tmp> serve` 后，模型写的 `app.py` 落到了**仓库根目录**而非临时 workdir——根因：`opencode_web::serve()` 用 `std::env::current_dir()` 作 workdir，`serve_run`/`serve_launch` 丢弃了 `cli.workdir`。即 `opencoder --workdir X serve` 静默服务 cwd，对用户是隐蔽 bug。
- 修复：`web::serve` 增 `workdir: PathBuf` 参数；`serve_run`/`serve_launch` 经 `resolve_workdir(cli)`（`--workdir` 优先、回退 cwd）传入。E11 现在正确隔离（app.py 进临时 workdir）。

## 涉及文件
- `crates/cli/src/lib.rs`、`crates/cli/src/session_cmd.rs`（+2 测试）、`crates/cli/tests/cli_parse.rs`（pattern 适配 + `--json` 解析测试）
- `crates/cli/src/serve.rs`、`crates/web/src/lib.rs`（serve --workdir 修复）
- `scripts/e2e/{__init__,lib,cli_scenarios,web_scenarios}.py`（新建）、`scripts/e2e_glm.py`（新建）、`scripts/e2e-glm.sh`（改薄 wrapper）
- `agents/cli/index.md`（新建）、`agents.md`（链接化 cli bullet）、`features/index.md`（e2e 描述）

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| `--json` 解析 | `session show --json` 解析 arm | `crates/cli/tests/cli_parse.rs` |
| 深度观测面（ToolUse 块存活） | `build_session_json_emits_meta_messages_and_subagent_tasks` | `crates/cli/src/session_cmd.rs` |
| 缺失 session 报错 | `build_session_json_errors_on_missing_session` | `crates/cli/src/session_cmd.rs` |

- 全量回归：`cargo test --workspace` → 296 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- e2e：`python3 scripts/e2e_glm.py` → 确定性契约（fork/bundle/resume/plan 只读/web delivery）全 hard-ok；模型依赖项（E1/E6 文件产出、E4 subagent 派遣）soft-skip

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| e2e 介质 | bash (e2e-glm.sh) | Python 包 (scripts/e2e/) + 薄 wrapper |
| e2e 断言深度 | 表面标记（grep/存在性） | 业务契约（hard/soft 分层） |
| e2e 场景数 | 10（9 + E9） | 12（+E10 plan 只读、+E11 web delivery） |
| CLI 观测面 | `session show` 仅 text | `session show --json` 全状态 |
| agents/cli 记忆 | 无（dangling bullet） | `agents/cli/index.md` |
