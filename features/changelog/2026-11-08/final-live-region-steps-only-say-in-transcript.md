# 收尾报告 · live-region-steps-only-say-in-transcript

## 裁决与结论归档

- 评审结论:**可上线(有条件通过)** —— 见
  [review-live-region-steps-only-say-in-transcript.md](review-live-region-steps-only-say-in-transcript.md)。
- 用户裁决:指示**完成收尾**,接受当前状态(运行观感由用户自行重启 TUI
  核验,未附反馈)。
- 测试结论:见
  [delivery-live-region-steps-only-say-in-transcript.md](delivery-live-region-steps-only-say-in-transcript.md)
  门禁事实段(定向范围,未跑全量,遵守 rules/02)。

## 最终 TODO 清单

- 无未决 TODO。
- 范围外遗留(明确推迟,非本轮目标):`theme.test.js` 1 例失败,归
  `21389c292` 的 brain/workbench/history/style.css。

## 线上验证清单 + 结论

| 项 | 结果 |
| --- | --- |
| `/usr/local/bin/opencoder` / `opencoder-server` 安装 cmp | 一致,版本 `3e609c7ef` |
| `oc-server` 重启后状态 | active,HTTP 200 |
| server 内嵌 SPA dist | `/static/app.js` 与 dist 字节一致 |
| 本地 dist 一致性 | `check-spa-drift.sh` no drift |

结论:**已上线** —— 服务端与二进制均运行本轮产物;TUI 为用户本地进程,
由用户重启后即与本轮一致(无代码阻塞项)。

## 对后续开发

- 实时区契约:仅当前运行 step 的 thinking + 阶梯同款调用行
  (`push_step_details(…, show_results=false)`),无 step 计数、无 Say;
  改动入口 `chat_live_turn.rs`,行为口径见本目录功能条目。
- 实时条几何不变量:定位用**可见**行数(`min(总行数, 框内高)`)、保尾按
  `render_viewport::wrapped_rows` 换行后行数 —— 违反任一条会复现
  「溢出不可见/底行丢失」(回归:`render_tests/live.rs:314`)。
- open Say 是转录内容:flatten 与 chat_headers 必须同源同步改
  (`rows_len` 口径),否则行数镜像/命中错位;旧「seal 才入转录」口径已废。
- SPA 侧 `TurnLiveRegion`/`StreamingSay` 须留在 transcript 滚动容器内
  (共用滚动条),auto-stick 仅近底生效。
