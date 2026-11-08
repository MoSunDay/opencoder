# 评审报告 · live-region-one-scrollbar-folded-say-ladder（b3ad08ad9）

> 评审对象：commit `b3ad08ad9`（18 文件，TUI 13 + 文档 5）。评审停在
> 逻辑层（未执行测试；门禁事实取自交付时实测）。证据仅取本轮产出：
> [功能变更](live-region-one-scrollbar-folded-say-ladder.md)、代码
> file:line、部署事实。

## 问一 · 原始需求与验收标准

1. live 区只渲染当前 step 的 thinking + function call（无 step 计数
   行、无 Say）。
2. Say 开始或 turn 结束（Done/Error）live 即刻清空。
3. live 与 Say/转录**共用同一条滚动条**：live 行并入滚动窗口尾部、
   永不截断（装不下出滚动条，上滚可见 live 开头）。
4. 流式 Say 是转录内容（共享滚动），但流式期间阶梯保持折叠
   `▸ N Steps`，`Say(n steps)` 合并仅 **seal 后**成立；SPA 同口径
   （本轮零改动）。

验收 = 1–4 静态逻辑闭合 + 定向门禁绿 + 文档收口 + 构建部署。

## 问二 · 改动清单（文件:行）

| # | 改动 | 位置 |
|---|---|---|
| 1 | 合账 `total_all = total_rows + live_rows`；follow/clamp/滚动条全按合账 | crates/tui/src/render.rs:464-465,468-475,553-554 |
| 2 | 一窗拼装：内容切片 + live 切片进同一 wrapping Paragraph（top_skip 吃部分首行） | crates/tui/src/render.rs:531-551、render_live.rs:30,40 |
| 3 | 旧条几何删除（strip_rect/render_strip/content_h 预留），全仓残留 0 | render_live.rs（删除）、render.rs（删除） |
| 4 | 流式 Say 独立渲染（`say:` 头行 + spinner + raw body 全量缩进 4、无去重） | crates/tui/src/chat_flatten.rs:50-83 |
| 5 | 合并仅 seal 后成立（`done: true` 匹配守卫，双侧同口径） | crates/tui/src/chat_flatten.rs:147、chat_headers.rs:59-65 |
| 6 | 行数镜像同步（open Say +1 头行 + raw 行数） | crates/tui/src/chat_headers.rs:83-85 |
| 7 | SayHeader 简化（streaming 字段/头行 spinner 删，helpers 只吃 rendered） | crates/tui/src/chat_step_render.rs:53,67,155,165 |
| 8 | 清空时机不变量保持（本轮未动状态机） | chat_steps.rs:131,332、chat.rs:347,366 |
| 9 | 测试（新增/重写/语义更新）+ 文档链 | render_tests/live.rs:448,269,129 等；changelog/README/tui index/repo-memory |

## 问三 · Blocker / 风险

- 代码级 Blocker：无。
- 非阻塞：用户并行会话 38 个未提交记忆快照文件（明确排除，未混入
  本 commit）；`theme.test.js` 1 例既有失败（`21389c292`，不在本轮
  链路，按定向回归口径不阻塞）。

## 问四 · 静态验证（逐项）

1. 统一滚动条：render.rs:465/477/538-551 —— `scroll_y ≥ total_rows`
   时内容切空、live 部分首行经 `live_top_skip` 由 `Paragraph.scroll`
   吃掉，与内容侧 `visible_window`（render_viewport.rs:236-260）同构。✔
2. live 永不截断、上滚可达开头：slice_window 预算只受窗口高约束
   （render_live.rs:55-79）；clamp 上限 `total_all − 可见高` 保证
   scroll_y 可达 0（回归 render_tests/live.rs:269）。✔
3. 边界：region 空 → wrapped_total=0 退化纯转录；budget=0 早返回空
   （render_live.rs:73-74）；clamp 保证 skip 不越界。✔
4. 折叠阶梯 + Say 独立：chat_flatten.rs:50 与 :147、chat_headers.rs:59-65
   与 :83-85 双侧镜像，行数口径一致。✔
5. 副作用面：live 仍 render-only 不进 `flatten_with`（render_live.rs
   模块契约）；hit-rect 仅记可见内容行（render.rs:484-514）；拷贝语义
   保留（copy_mode/clean_say_tests.rs:122）。✔
6. 陷阱自检四条：只声明未做=否；偷换概念=否（「不截断共滚动条」「seal
   后才合并」逐字兑现）；删功能=否（折叠/计数/清空/复制全保留，删的是
   被取代的几何实现）；绕开测试=否（交付时定向门禁实测全绿：TUI lib
   1813/1813——含 bash_exec 环境误报修正后复验、clippy 0、fmt clean、
   SPA no drift）。✔

## 问五 · TODO

- 用户重启 TUI 运行时核验（后已通过，见 final 报告）——非阻塞。
- `theme.test.js` 既有失败：范围外延期 —— 非阻塞。
- render_tests/live.rs:30 等测试名仍沿用 "strip" 措辞（断言已对齐统一
  窗，仅命名陈旧）：可选清理 —— 非阻塞。

## 上线结论

**可上线**：五问闭环、陷阱自检四项全过、无阻塞卡点；定向门禁全绿，
双二进制已构建安装（b3ad08ad9），oc-server active/HTTP 200，SPA 无
漂移。
