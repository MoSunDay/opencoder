# 评审报告 · live-region-steps-only-say-in-transcript(3e609c7ef)

> 评审对象:commit `3e609c7ef`(28 文件,TUI + SPA + 文档 + dist)。
> 证据仅取本轮产出:[功能变更](live-region-steps-only-say-in-transcript.md)、
> [被取代条目](live-region-hugs-content-say-deferred.md)、本轮门禁与部署事实。

## 问一 · 评审范围与原始需求

1. 实时区**无 step 计数行**,仅当前 step 的 thinking + function call 行。
2. 修复实时条不可见/显示异常 bug。
3. 实时区**不含 Say**。
4. turn 终止即清空实时区。
5. 流式 Say 作为转录内容进滚动区,与实时区共用滚动。
6. 交付链:release 构建 + 安装 `/usr/local/bin` + 重启 `oc-server` 内嵌新 dist。

验收口径 = 1–5 的静态逻辑闭合 + 门禁绿 + 部署事实。

## 问二 · 改动清单(文件:行)

| # | 改动 | 位置 |
|---|---|---|
| 1 | `live_data` 仅 `progress_active` 组;`render_lines` 只产 `⬥ live`+thinking+调用行 | crates/tui/src/chat_live_turn.rs:35,56 |
| 2 | 实时条贴**可见**内容窗(`total_rows.min(visible_h)`),修复溢出外推 | crates/tui/src/render.rs:559(几何 464–483) |
| 3 | 保尾按 `wrapped_rows` 实际行数 + `.scroll`,物理溢出才裁剪 | crates/tui/src/render_live.rs:55,61 |
| 4 | open Say 即时入转录最终形态(合并不看 done、raw body、`Skip` 下标保护) | crates/tui/src/chat_flatten.rs:137,73,80 |
| 5 | 行数镜像同步(任意 Assistant 合并、`rows_len` 同源口径) | crates/tui/src/chat_headers.rs:63,94–103 |
| 6 | 清空时机:Say 开始 / Done+Error 解除 progress | crates/tui/src/chat_stream.rs:22、chat.rs:347,366 |
| 7 | 旧 API 调用点迁移(转录取证) | app_loop_tests/sidecar_display_tests.rs、remote/tests/{mod,protocol}.rs |
| 8 | SPA 同口径:TurnLiveRegion 去 say、StreamingSay 入滚动容器、近底 auto-stick | stepsBlock.jsx:240,272;chat.jsx:698–712,75,337 |
| 9 | 回归(含 `live_strip_stays_visible_when_transcript_overflows`)+ 文档链 | render_tests/live.rs:314 等 14 测试文件 |

## 问三 · Blocker / 风险

- 代码级 Blocker:无。
- 范围外遗留:`theme.test.js` 1 例失败源自 `21389c292` 的
  brain/workbench/history/style.css,本轮未触碰、不阻塞(明确推迟)。

## 问四 · 静态验证(逐项)

1. steps-only:chat_live_turn.rs:35/56 → `push_step_details` 仅 thinking+聚合+头行。✔
2. 可见性:`条y ≤ inner.y+content_h` 且 `条高余量 ≥ live_len`(content_h>0);
   `content_h==0` 整框尾保分支不回归(render.rs:478)。✔
3. 保尾:wrapped 求和 + scroll,与内容区同技法,无换行丢行。✔
4. Say 入转录:开放 follower 即合并(头行+running),body 流式 raw、seal 仅换
   rendered 并去提示 —— 零跳变;headers 计数同源无下溢;渲染/计数/命中/复制
   同走 flatten+headers,无分叉。✔
5. 清空:Say 开始(chat_stream.rs:22)与终止(chat.rs:347/366)解除 progress
   → `live_data` None → 区域即空。✔
6. 调用点迁移:3 文件 4 处全改,旧签名 grep 零命中。✔
7. SPA:滚动容器内 StreamingSay→TurnLiveRegion 顺序;auto-stick `<80px` 保护。✔
8. 行数:render.rs 799/800、chat.jsx 800/800、changelog 53 行。✔
9. 门禁(本轮已执行):TUI lib 1812/1812、clippy --all-targets 0、fmt clean;
   SPA 定向 44/44;dist 无 drift。✔

## 问五 · TODO / 待确认(收尾前)

- 用户侧 TUI 重启终验(部署动作)。
- [已移除] SPA auto-stick 手感确认 —— 用户未裁决,按规则不保留。

## 上线结论

**可上线(有条件通过)** —— 需求 1–5 逻辑闭合、门禁全绿、二进制与 server 已
部署验证;唯一余项为用户重启 TUI 的运行终验,无代码阻塞。
