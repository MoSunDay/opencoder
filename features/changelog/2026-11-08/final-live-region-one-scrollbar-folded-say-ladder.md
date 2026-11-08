# 收尾报告 · live-region-one-scrollbar-folded-say-ladder

## 裁决与结论归档

- 评审结论：**可上线** —— 见
  [review-live-region-one-scrollbar-folded-say-ladder.md](review-live-region-one-scrollbar-folded-say-ladder.md)。
- 用户裁决：**TUI 渲染已验证正常**（重启后运行时核验通过），并确认
  **web 交互逻辑与 TUI 完成对齐**（SPA 本轮零改动，口径本就一致：
  TurnLiveRegion/StreamingSay 在 transcript 滚动容器内共用滚动条、
  流式期阶梯折叠、flush/seal 后落地合并）。
- 测试结论：见
  [live-region-one-scrollbar-folded-say-ladder.md](live-region-one-scrollbar-folded-say-ladder.md)
  定向回归段（TUI lib 1813/1813、clippy 0、fmt clean、SPA no drift；
  定向范围，未跑全量，遵守 rules/02）。

## 最终 TODO 清单

- 无未决 TODO。
- 范围外遗留（明确推迟，非本轮目标）：`theme.test.js` 1 例失败，归
  `21389c292` 的 brain/workbench/history/style.css。
- 可选清理：`render_tests/live.rs` 若干测试名仍沿用 "strip" 措辞
  （断言已对齐统一窗口，仅命名陈旧）。
- 用户并行会话的记忆快照文件由用户自行提交（未混入本轮 commit）。

## 线上验证清单 + 结论

| 项 | 结果 |
| --- | --- |
| `/usr/local/bin/opencoder` / `opencoder-server` | 已安装，版本 `b3ad08ad9` |
| `oc-server` 状态 | active，HTTP 200（本轮无 dist/server 改动，按「dist 变更才重启」口径未重启） |
| SPA dist 一致性 | `check-spa-drift.sh` no drift |
| TUI 运行时 | 用户重启核验：渲染正常 |
| SPA 交互 | 用户确认与 TUI 对齐 |

结论：**已上线** —— 二进制运行本轮产物，TUI 经用户运行时核验，web
交互口径经用户确认对齐（无代码阻塞项）。

## 对后续开发

- 统一窗不变量：`total_all = total_rows + wrapped_total(live)` 是滚动
  条 / follow / clamp 的**唯一账本**；live 切片的 `skip→top_skip` 与
  内容侧 `visible_window` 同构（render.rs:531-551）—— 违反会复现
  截断 / 错位（回归：`render_tests/live.rs:448,269`）。
- 合并时序：`done: true` 匹配守卫在 `chat_flatten.rs` 与
  `chat_headers.rs` **双侧同源**，改动必须同步，否则行数镜像错位。
- 不要复活 strip 几何：`strip_rect` / `render_strip` / `content_h`
  预留分支已删，live 一律走滚动窗尾部拼装。
- hit-rect 只记可见内容行（`content_vis`），live 尾无点击目标；live
  保持 render-only，不进 `flatten_with`。
- SPA 侧 `TurnLiveRegion` / `StreamingSay` 保持在 transcript 滚动容器
  内（共用滚动条），auto-stick 仅近底生效；与本条目口径同步演进。
