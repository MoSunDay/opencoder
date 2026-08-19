# autopilot verify 判定收紧、judge 故障成因可区分、review_pass 入口取消检查

## 背景

autopilot 模块调查确认 4 项代码缺陷 + 2 处 cosmetic，本轮修复；另有一项守卫跨迭代
不累计的行为经用户决策**不改代码、只修记忆表述**（见下「明确不做」）。

## 变更

- **verify 判定严格单 token**（`crates/session/src/autopilot/decision.rs`）：
  `parse_verdict` 由「首词启发式」改为**整串严格匹配**——trim 外围空白 + 尾部一组
  句读标点后，整个回答必须恰为一个可接受 token（大小写不敏感，容忍
  `yes`/`y`/`true`/`1`/`是` 及否定对偶）。限定性回答（"Yes, but the tests fail"）
  不再被误判为 Complete，而是 `None` → Malformed → `verify_retries` 内重试。
  `verify_user_prompt`（`prompts.rs`）同步强化单 token 约束措辞。
- **judge 传输错误与不可解析区分**（`verify.rs` + `state.rs` + `decision.rs`）：
  `verify()` 返回值由 `VerifyVerdict`（Malformed 折叠一切）改为
  `Result<VerifyVerdict, VerifyFailure>`；`VerifyVerdict::Malformed` 删除，新
  `VerifyFailure::{Unparseable{attempts}, Unreachable{attempts, last_error}}`
  携带尝试次数与最后一次传输错误原文。预算内两类失败照常重试；耗尽后
  `should_stop` 生成**成因可区分**的 `Aborted` reason（
  `verify verdict unparseable after N attempts` vs
  `verify judge unreachable after N attempts (last error: …)`）+ `tracing::warn`。
  **不新增** ApOutcome/SessionEvent 变体，TUI/CLI/web 渲染面零波及。
- **review_pass 入口 pre-flight cancel 检查**（`review_pass.rs`）：入口对照
  `drive` 的 loop-top 检查补 `is_cancelled` 预检——已取消则零副作用返回（不发
  AutoPilot 事件、不切 plan agent、不注合成 review prompt、不调 LLM），仅执行与
  drive 取消路径相同的终态记账（`clear_injected_skill` + `Done`），消除「边界处
  取消把 session 留在 plan agent + 残留合成 prompt」的窗口。
- **`/config` 表单 u32 溢出拦截**（`crates/tui/src/model_menu/config_form.rs`）：
  `validate()` 对 `fps_input`/`ap_max_iter_input` 增加 `parse::<u32>` 检查
  （与 threshold/context_size 同风格 "is not a number" 错误），拦截
  `4294967296` 类溢出串；此前 validate 只查空串，溢出串静默经 `build_patch` 的
  `unwrap_or(10)` 回落 10。`unwrap_or` 保留为直接调用方的 safety net。
- **cosmetic**：`prompts.rs::review_prompt` 去掉续行符事故造成的字面缩进（换行后
  9 空格）；`command.rs` `/ap` doc 注释由 "toggle autopilot" 更正为「打开模式
  选择菜单」（行为本就如此，无测试钉住旧注释）。

## 记忆同步（repair-on-touch）

- `features/index.md`：E18 描述的 config 合并键名由旧 `enabled` 更正为
  `{"mode":"ap"}`；autopilot 三态条目补「判定严格单 token」「Aborted reason 区分
  unparseable/unreachable 成因」；守卫句由「约束不被绕过」更正为「doom-loop /
  tool-failure guard 按**单次 run（单个 phase）作用域**累计，phase/迭代边界重置、
  跨迭代不累计，整轮由 `max_iterations` 兜底；cancel 在 drive 循环边界检查
  （`review_pass` 入口亦有 pre-flight 取消检查）」。
- `agents/session/index.md`：review_pass 描述补入口 pre-flight cancel 语义。
- 不新建 `agents/ap/` 记忆文件（横切特性，寄居现状符合 Reliability Beats Coverage）。

## 明确不做

- doom/tool-failure 守卫跨 autopilot 迭代不累计（每 phase 新开 run_loop 即清零）：
  用户决策不修代码，记忆已如实标注按单 run 作用域、`max_iterations` 兜底。
- 空 prompt drain 窗口、Review 迭代号 1 vs 0（有意）维持现状。

## 测试清单

- `session` 单元（`src/autopilot/tests.rs`）：`parse_tolerates_punctuation_and_whitespace`
  （翻转：不再吃 "Yes, more work"）、新增 `parse_qualified_answers_are_malformed`、
  `malformed_aborts` → `unparseable_exhaustion_aborts_with_cause` + 新增
  `unreachable_exhaustion_aborts_with_cause`（断言 reason 含成因与次数）。
- `session` 集成（`tests/autopilot.rs`）：`verify_garbage_retries_then_unparseable`
  （原 malformed 用例改判 Err(Unparseable{attempts:3})）、新增
  `verify_transport_errors_report_unreachable`（Err(Unreachable)+last_error）、新增
  `verify_qualified_yes_is_unparseable_not_complete`、新增
  `drive_aborts_when_verify_judge_is_unreachable`（reason 含 unreachable + 传输错误）、
  `drive_aborts_when_verify_keeps_malformed` 补 reason 含 unparseable 断言；
  verify yes/no/snapshot 用例改 `.unwrap()`。
- `session` 集成（`tests/autopilot_review.rs`）：新增
  `review_pass_cancelled_at_entry_is_a_no_op`（0 LLM 调用、agent 不切换、无合成
  消息、无 AutoPilot/AgentSwitch 事件、恰一个 Done）。
- `tui`（`model_menu/tests/config_tests.rs`）：新增
  `validate_rejects_fps_u32_overflow`、`validate_rejects_ap_max_iter_u32_overflow`
  （4294967296 / 99999999999999，Idle + 菜单保留 + error 文案）。
- e2e：`scripts/e2e/cli_scenarios.py` E18 / `web_scenarios.py` E18b 断言审阅确认
  不受 verify 收紧影响（phase 标记与 Done 在 Aborted 路径同样发出；judge 由真实
  glm5.2 扮演）。本轮无模型 key，**未实跑**，已在本文标注。

## 回归

- `cargo test -p opencoder-session --test autopilot --test autopilot_review --test autopilot_skill_persist`：32 通过。
- `cargo test -p opencoder-session --lib`：385 通过（后续并入 verify_context_limit 迭代后复验 395 通过）。
- `cargo test -p opencoder-tui`：在隔离 worktree（HEAD + 本轮 2 文件）全绿 1450 通过（含 2 个新增溢出用例）；
  主树当时正被并发迭代占用，见下「并发说明」。
- `cargo test -p opencoder-core --test config_autopilot_contract`：4 通过。
- `cargo clippy`：`-p opencoder-session -p opencoder-tui -p opencoder-cli -p opencoder-core --all-targets -D warnings`
  全净；早前全 workspace clippy 亦绿（含顺带机械修复 `cli/session_cmd.rs` 的 `map_flatten` lint）。
- 全量 `cargo test --workspace`（rules/02）：**阻塞于并发迭代面**，见下。

## 顺带修复（回归中发现的既有缺口）

- `crates/session/tests/compound_cmd.rs` 3 个用例：前一轮「one-shot `$skill`」迭代引入 run 结束清 skill
  （`skill_lifecycle::run_loop_one_shot`）但未同步更新这三个断言「run 后 skill 仍存活」的旧测试（其自身
  契约测试 `skill_one_shot.rs` 6/6 通过，意图明确）。已按 one-shot 语义修正：激活证明改为「该 run 的 LLM
  请求携带 `[skill loaded]`/`[active skill]`」，run 结束断言已清除。修正后 5/5 通过。
- `crates/session/tests/{autopilot,autopilot_skill_persist}.rs` 的 `AutoPilotConfig` 字面量补
  `..AutoPilotConfig::default()`（并发迭代新增 `verify_context_limit` 字段的 future-proof，仓库既有约定）。

## 并发说明（全量门未闭合的归因）

本轮执行期间同一工作树存在**并发迭代者**（证据：文件在两次读取间被改写、周期性 `cargo test` probe 进程、
`75d6866` 提交把本任务未提交改动一并卷入并继续在其上开发 `verify_context_limit`/`REVIEW_ITERATION 0-based`/
tui `UiCmd` 重构）。截至本收尾：
- `crates/todos/tests/drive_degrade.rs`：并发者 in-flight 测试，编译错→逻辑红，文件持续变动，不属本任务范围。
- `crates/session/tests/plain_skill_prompt.rs` 与 tui `UiCmd` 编译错：同为并发者 00:04 后未提交的中途态。
- 本任务全部定向套件 + session lib + tui（隔离验证）+ core 契约 + 4 crate clippy 在并发改动并入后复验全绿。
