Commit: (working-tree, post-1ba8f426)

# task-plan question 工具澄清指引

## Context

`question` 工具的 schema 仅投递给 plan agent（见 `crates/session/src/tools/mod.rs::question_schema_is_plan_only_and_compact`），而 task-plan 正是 plan 模式的规划契约——却没有任何使用该工具的指引：目标含糊时要么自行脑补验收标准、要么整轮停下。本轮补齐条件式澄清协议（沿用 do-and-done 暂停协议的写法）：交互可用即问、headless 显式假设。

## Change Summary

- `crates/core/assets/skills/task-plan/SKILL.md`：
  - 新增「澄清协议（目标含糊 / 需求冲突时）」小节：仅**会改变拆解方向**的真歧义才触发（能从仓库 / `rules/` / 既有测试查到的事实先查再定，不把提问当侦察手段）；`question` 工具可用（plan agent 交互式 TUI）→ 每次一句一个最关键问题、可附 ≤4 候选选项、同轮可多问；不可用（非交互 `run` / headless，工具即刻返回「无监听」应答不阻塞）→ 显式假设继续：逐条写入 STATUS 块 `assumptions:`、选最小意外解释、gate 汇报标注「规划基于假设」，绝不静默编造验收标准。
  - STATUS 块模板增补可省略的 `assumptions:` 字段（承接 headless 分支的假设落点）。
  - 规划四步法第 1 步「目标澄清」补一句指针：真歧义按澄清协议处理，不自行脑补。
- `crates/core/tests/skill_contract.rs`：新增 `seeded_task_plan_skill_requires_question_tool_guidance`——seed 后断言 SKILL.md 含 `question` 引用、`澄清协议` 小节、两分支措辞（`question` 工具可用 / 不可用）、`assumptions:` 落点与「不把提问当侦察手段」防偷懒守卫；asset 被改丢即红。
- 记忆文档 repair-on-touch：`features/index.md` 内置 skill 清单 task-plan 括注补「真歧义经 question 工具澄清，headless 下显式假设写入 STATUS」。
- **seeding 语义不变**：资产 `include_str!` 内嵌、首启 per-file seed 且 never-clobber，老用户已存在的文件不会被覆盖。
- 附带收尾（本轮工作树内其他半成品迭代补完）：`crates/tui/src/terminal.rs::consume_modifier_or_release` 补齐调用点已传入的 `copy_mode` 参数（copy mode 期间抑制 Shift 按/放对鼠标捕获的抢占，防 Shift 释放把捕获抢回、破坏终端原生选择）+ 对应单测；`crates/tui/src/notepad/editor.rs::row_texts` 补硬换行回填（wrap range 跳过的 `\n` 重新插入，拼接行可精确重建 buffer），`copy_mode.rs::render_notepad_clean` 渲染时 trim 回去；`crates/session/tests/autopilot.rs`（847 行超限）按功能边界拆出 `autopilot_skill_persist.rs`（123 行，persisted-skill 生命周期测试），本体回到 772 行；全仓 `cargo fmt`。

## Validation（当次实跑）

- `cargo test --workspace --no-fail-fast`：175 个套件全绿，合计 **2855 passed / 0 failed**（上一基线 2804，净增 51：本轮 +1 skill_contract，其余为工作树内 drain/tui/web 等迭代的既有新增）；`-p opencoder-core --test skill_contract` 16 passed。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（exit 0）。
- `cargo build --workspace`：编译干净（Finished dev profile）。
- `cargo fmt --all --check`：干净。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `skill_contract.rs::seeded_task_plan_skill_requires_question_tool_guidance` | integration | seed 后 task-plan SKILL.md 含 question 引用 + 澄清协议两分支 + assumptions 落点 + 防偷懒守卫——asset 字段被改丢即红 |
| `terminal.rs::consume_modifier_tracks_shift_in_copy_mode_without_capture_fight` | unit | copy mode 下 Shift 状态机照常（事件被消费、shift_held 正确翻转），仅捕获切换被抑制 |
| `notepad::editor::tests::row_texts_round_trips_and_wraps_in_order` | unit | 行文本拼接精确重建 buffer（含硬换行）、窄宽强制多行、空 buffer 单空行 |
| `autopilot_skill_persist.rs::drive_clears_persisted_skill_on_complete_and_on_error` | integration | autopilot Complete/phase-error 两种收场都清内存 + 持久化 skill（迁移自 autopilot.rs，未改语义） |

纯 markdown asset + 契约测试 + 半成品收尾，无删测试 / 无 `#[ignore]` / 无弱断言 / 无密钥。
