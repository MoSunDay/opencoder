Commit: f483b8c

# 真机浏览器验收：Say 配对三场景自包含闭环（mock LLM）+ chat.dom 卫生修复 + 溯源报告落盘

## Context

[评审跟进](../2026-09-03/review-followup-say-pairs-gates.md) 遗留 3 项 TODO：①真机浏览器验收（P2，当时缺 playwright-core / 真 LLM 长流）；②`chat.dom.test.jsx` 两行卫生修复（P2，当时并发会话所有、仅记录修法移交）；③loop-report 要点落盘（P3，`/tmp` 易失）。本轮工作区干净、所有权约束解除，三项全部闭合。

## Change Summary

- **真机验收自包含闭环（TODO-1）**：新增 `scripts/browser-acceptance-saypairs.js`（400 行）+ `scripts/mock-llm-saypairs.js`（240 行）。运行时生成 token/端口/临时 workdir，配置指向本地 mock OpenAI 兼容 SSE 服务（仅 dummy key，零真实凭证），spawn `opencoder daemon --server` + 系统 chromium（playwright-core 列为 spa devDependency，无浏览器下载），沿用 `browser-acceptance.js` 手法（HMAC signedJson、step/shot/SUMMARY、exit code）。连续 3 次全绿（每次约 25s），10 张证据截图落 `/tmp/uitest/shots-saypairs`。
- **三场景断言**（上轮人工 TODO 的真机化）：(a) 两回合 `[❯ 1 Step + Say]×2` 严格 DOM 序（echo→ladder→echo→ladder，`compareDocumentPosition`）、活跃期 running Tag `margin-left:12px`、Say wrapper `margin-top:8px`、L0→L3 钻取（`Step(k)`→`Function call(s)`→`🔧 bash` + `output: hi N`）；(b) `/act_clear_context 收尾总结` 在途回显恰显尾巴、`transcript_reset` 快照重构后幂等存活（恰一次、done 后为最后一气泡）；(c) steer 拆梯。
- **契约修正（实测发现，已回写 [agents/web](../../../agents/web/index.md)）**：web 运行中提交的 steer 是 **turn 级中断**——签名 steer POST 立即 `fire_turn_cancel`（`crates/web/src/handle.rs` "Steers interrupt the current turn"），在途回合 tool_end 标 `turn interrupted`、未完成的部分 Say 文本不落库，steer 本身在下个 turn 边界吸收。帧级 e2e 的「steer 消费开新梯」是 drain 内部语义；真机 (c) 按实测契约断言：梯 A 冻结于 `❯ 1 Step`（被截断 Say 丢弃）、echo-B 之下梯 B running→Say-B。另两处真机适配：用户回显渲染带 TUI-parity `❯ ` 前缀（exact-match 需剥离）；done 后整段历史重建（mid-run 视图仅含当前 run），梯定位一律用结构锚（echoX 上/下梯）而非索引。
- **`chat.dom.test.jsx` 卫生修复（TODO-2，移交项落地）**：`beforeEach` 补 `sessionSnapshots = {}`（router fixture map 逐测重置）与 `consoleLog.error/warn` 桶清零（原数组上清长度保 spy 身份，deprecation gate 从「靠声明顺序幸免」变 per-test）。
- **loop-report 落盘（TODO-3）**：归因结论沉淀——23 次全量 vitest 仅 run #12/#13 失败；porcelain/diffstat 均为路径/状态哈希，对「同路径内容重写」不可见，靠 mtime 闭合归因为并发子代理恰在循环中重写 `saypairs.e2e.test.js`（非套件污染）；计数漂移 179→186 全由两个新测试文件解释。归因方法（路径哈希盲区 + mtime 闭合）供后续瞬态失败排查复用。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| 真机 (a) 多回合交替 + 12px/8px 间距 + 钻取 | `scripts/browser-acceptance-saypairs.js` a1/a2/a3（新增） |
| 真机 (b) `/act_clear_context` 在途回显存活恰一次 | 同上 b1/b2（新增） |
| 真机 (c) steer 拆梯：旧梯冻结、新梯在 echo 之下 | 同上 c1/c2（新增） |
| mock LLM wire 契约（finish_reason 必带、tool_calls 增量拼接、usage 帧） | 同上脚本 3 连跑 exit 0 + mock 决策日志 |
| fixture map / 告警桶逐测重置 | `spa/src/chat.dom.test.jsx` 12 例回归 |

## 回归

`cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets` 0 警告；`cargo test --workspace` 全绿（3997 passed / 0 failed）；spa vitest 186/186（17 文件）；`scripts/check-spa-drift.sh` no drift（devDependency 不入 dist）；验收脚本交付后独立复跑 2 次 exit 0（8/8 PASS）。
