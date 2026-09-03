Commit: 7a09d29

# 验收脚本硬化：启动段纳入 try/finally、mock 断流可观测、固定 sleep 换确定性探针

## Context

[上轮评审](browser-acceptance-saypairs-mock-llm.md) 结论 ready，遗留 3 项 P3 TODO：①验收脚本 mock/daemon/browser 启动段在 `try` 之外（启动失败绕过 `finally`，泄漏 detached daemon）；②mock 流写错误被 `catch{}` 吞（`[DONE]` 前缺 finish 帧时客户端 Truncated 重试，故障面具化）；③done→重建的固定 `sleep(400/500)` 换确定性探针、`consoleErrors` 只汇总不断言。另含措辞收紧：`agents/web/index.md` 的「tool_end 标 interrupted」与 `runner/execute.rs` 实际标记串 `turn interrupted` 对齐。

## Change Summary

- **启动段收进 try/finally（TODO-1a）**：`try {` 上移到 `startMock` 之前，启动失败同样走 `ABORT` 日志 + `finally` teardown；`browser.close()` 补 null 守卫（启动早期失败时 browser 尚不存在）。以 `CHROME_PATH=/nonexistent` 实测：exit 1 + ABORT + mock/daemon 全部回收、workdir 清除、零孤儿进程。
- **mock 断流可观测（TODO-1b）**：实测发现 Node 对已断开 response 的 `write` **静默返回 false 而不抛错**——原 `catch{}` 对 client-abort 是死代码。改为三层记录：`res.on('close')` 在帧未写完时记 `client_gone_before_finish`（含 plan tag，真实断流信号）、`catch (e)` 记 `stream_write_error`、`res.on('error')` 记 `res_error`（兼防未处理 error 事件崩掉 standalone 进程）。流仍以无 finish 帧的 `[DONE]` 收尾 → 客户端 Truncated，两侧都可诊断。一节式验证：mid-slow-Say abort → 恰一条 `client_gone_before_finish`、mock 存活继续服务；干净请求零误报（finish_reason + [DONE] 齐全）。
- **固定 sleep 换确定性探针（TODO-3a）**：`doneRebuild(signed, timeout)` = 基线计数 → `waitDraining(false)` → 等「浏览器又发过一次 `GET /api/sessions/:id`」（`page.on('response')` 按 `/\/api\/sessions\/[^/?]+$/` 计数，排除 `/seq`/`/events`/list）→ 双 rAF 等 React 提交。a1/a3/c2 换用之。
- **b2 探针返工（本轮实测发现）**：首版 fetch 计数探针在 b2 稳定失败——空 Say 无 `waitText` 锚点，且整条 done 尾巴（echo→reset→done→reload，mock 上 ~50ms）可整个落在 `waitUserEcho` 150ms 轮询间隔内，基线已含 done-reload。DB 取证（`session_events` seq 34-39：`steer_consumed→transcript_reset→done` 齐全）确认**非产品缺陷**，是探针设计伪影。b2 改为轮询断言终态本身（`waitEchoSettledLast`：echo 恰一次 + 为末气泡 + 无 running tag + Sender loading 消失），b2 从固定 500ms 变 55-89ms 实测收口。a1/a3/c2 的锚点确定性有结构性保证：mock `writeSay` 末块后仍有 ≥300ms sleep > 200ms 轮询间隔，已写入 `doneRebuild` 注释。
- **consoleErrors 计入退出条件（TODO-3b）**：`exit 0 = 全 PASS 且 consoleErrors 为空`（非空时先打 WARN 行）；基线 5 次全绿运行 consoleErrors 均为 `[]`，无良性噪音误伤。
- **措辞收紧（TODO-2）**：`agents/web/index.md` 「其 tool_end 标 interrupted」→「标 `turn interrupted`」（与 `runner/execute.rs:330` 字面一致）。

## 测试覆盖（规则 01）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 启动失败也走 teardown（无孤儿 daemon/mock、workdir 清除） | `CHROME_PATH=/nonexistent` 启动失败路径实测（exit 1 + ABORT + 零孤儿） | `scripts/browser-acceptance-saypairs.js`（人工触发，非自动化） |
| client 断流可观测且不崩 mock | 一节式 node 验证：abort 恰记 1 条 `client_gone_before_finish`、mock 存活、干净请求零误报 | `scripts/mock-llm-saypairs.js`（人工触发） |
| done→重建确定性收口（a1/a3/c2）+ b2 终态轮询 | 8 步全 PASS × 5 次独立运行 exit 0 | `scripts/browser-acceptance-saypairs.js` |
| consoleErrors 计入退出条件 | 同上（consoleErrors==[] gate） | 同上 |

## 回归

`cargo fmt --all -- --check` 干净；`cargo build --workspace` 零错误；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo test --workspace` 全绿（3997 passed / 0 failed，与基线一致，本轮零 Rust 产品代码变更）；spa vitest 186/186（17 文件）；`scripts/check-spa-drift.sh` no drift；验收脚本改动后独立复跑 5 次全 8/8 PASS exit 0。行数 gate：`browser-acceptance-saypairs.js` 438 行、`mock-llm-saypairs.js` 257 行（均迭代中 ≤800）。
