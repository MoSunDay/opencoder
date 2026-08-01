# test(e2e): autopilot 真实模型覆盖 E18（CLI 全链路）+ E18b（web SSE 事件流）

## Summary

为 autopilot（`crates/session/src/autopilot`，`config.autopilot.enabled`）补齐第三层
e2e 覆盖：此前 ap 只有 Rust 单测/集成测（MockChatClient），真实 glm5.2 端到端为零。
本轮新增两个场景，用真模型实跑验证 ap 全链路（初始 turn → PLAN → ACT → VERIFY → Done）：

1. **E18（CLI，`cli_scenarios.py`）**：autopilot 配置（`enabled=true, max_iterations=1,
   verify_retries=1`）下跑 headless prompt；断言 config 合并 JSON 携带 ap 配置、
   display.rs 阶段标记（`autopilot: Plan/Act/Verify (iteration 0)`）、注入的 phase
   prompt 落库、以及反向守卫（默认关闭时 E1 日志无 `autopilot: ` 标记）。
2. **E18b（web SSE，`web_scenarios.py`）**：独立 serve 实例（不复用 E11/E15 的实例，
   隔离 ap 多轮对 interrupt 时序的干扰）+ autopilot 配置，POST steer prompt 后读
   `/events?after=0` SSE，断言 `event: autopilot` 的 phase 序列 plan→act→verify
   （iteration 0，JSON 小写）并以终态 `event: done` 收尾。

关键契约事实（代码级确认）：

- 阶段事件在 `drive()` 一经启动必发（`runner/event.rs` `SessionEvent::AutoPilot`，
  `sse_kind="autopilot"`，`EventKind::Step` 持久化 → SSE replay 可靠）。
- 每个 `run_loop` idle 边界都发一次 `done`（初始 turn / PLAN 阶段 / ACT 阶段各一次），
  只有 VERIFY 之后 `autopilot::finish` 发的 `done` 才是终态 —— E18b 读流逻辑以
  「最后一个 autopilot(verify,0) 之后的 done」作为终止条件。
- 注入 prompt 落库契约：PLAN continuation（`"Autopilot PLAN phase"`）在 handoff 前
  经 `session.record` 持久化，两种路径（handoff / fallback）都必在 store；handoff
  消息（`"Planning phase complete."`）是内存态（`after_handoff` store 记账，见
  `session/src/lib.rs` + `tests/autopilot.rs:642`），断言取三串并集，HARD 且确定性。
- `max_iterations=1` 经 drive clamp 后恰一轮 → 恰好一个 Plan/Act/Verify(iteration 0)
  三元组。

## Changes

### `scripts/e2e/cli_scenarios.py`
- 新增 `AP_PROMPT`（创建 `hello_ap.txt` + cat 验证，一轮可完成的小任务）。
- E17 块后新增 **E18**：`autopilot` 配置 workdir → headless 运行 → HARD 断言
  （config_show 合并 JSON `autopilot.enabled==true` / `max_iterations==1`；
  log 含 `autopilot: Plan/Act/Verify (iteration 0)`；show_json 含注入 phase prompt
  并集；E1 日志反向守卫无 `autopilot: `）+ SOFT 断言（`hello_ap.txt` 产物、
  `meta.handoff_plan` 落库）；`sid is None`（模型/网络瞬断）按既有惯例 soft-skip。
- docstring 契约深度表补 E18 行。

### `scripts/e2e/web_scenarios.py`
- 抽出 `_boot_serve` / `_shutdown` 助手（serve 启动 + 健康等待 + 终止），供 run_all
  与 E18b 共用；docstring 补 E18b 行。
- 新增 `_run_e18b_autopilot`：独立 serve（autopilot 配置）→ POST steer prompt →
  SSE `readline` + `socket.timeout` 兜底 keep-alive 间隙，读至终态 done →
  HARD 断言 phase 序列 `[("plan",0),("act",0),("verify",0)]` + 终态 done；
  error 事件 / EOF / deadline → soft-skip。

### `features/index.md`
- e2e 覆盖清单追加 E18 / E18b 一条。

## 测试覆盖

| 场景 | 断言契约 | 位置 |
| --- | --- | --- |
| E18 config 合并 | `config show` JSON 含 `autopilot.enabled==true` / `max_iterations==1` | `scripts/e2e/cli_scenarios.py` |
| E18 阶段标记 | log 含 `autopilot: Plan/Act/Verify (iteration 0)`（display.rs 精确输出） | 同上 |
| E18 注入 prompt 落库 | show_json 含 PLAN/ACT/handoff 注入串（三取一并集，确定性） | 同上 |
| E18 反向守卫 | 默认配置 E1 日志无 `autopilot: `（关闭语义） | 同上 |
| E18 产物 | `hello_ap.txt` 存在（SOFT） | 同上 |
| E18 handoff 元数据 | `meta.handoff_plan` 落库（SOFT） | 同上 |
| E18b SSE 阶段流 | `event: autopilot` phase 序列 plan→act→verify（iteration 0） | `scripts/e2e/web_scenarios.py` |
| E18b 终态 | VERIFY 之后收到 `event: done`（error/EOF/超时 → soft-skip） | 同上 |

- 全量回归：`cargo test --workspace` → **1592 passed / 0 failed / 1 ignored**（当次实跑；基线 1587 + 5 新增来自同批 TUI `/ap` 功能测试；ignored 为既有 `research_smoke_bing_wikipedia`，需真实 Chrome/网络）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- e2e（真实 glm5.2）：`scripts/e2e-glm.sh --only cli` → 61 passed / 0 failed / 0 skipped（含新 E18，真模型实跑）；`--only web` → 21 passed / 0 failed / 0 skipped（含新 E18b，独立 serve + SSE 实跑）
