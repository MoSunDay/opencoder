# 评审报告 · TUI TranscriptReset 重建离环+压缩去在途回显+客户端发布 685ec8b09(基线 685ec8b09 + 未提交工作区)

> 评审对象：基线 commit `685ec8b09` + 未提交工作区(三个需求交付：2×TUI 变更 + 1×客户端发布)。
> 证据仅取本轮产出：[重建离环](../2026-11-16/tui-transcript-reset-offloop.md)、
> [压缩去在途回显](../2026-11-16/tui-compaction-no-inflight-echo.md)(反转
> [2026-11-06 口径](../2026-11-06/tui-compaction-inflight-echo.md),历史不回改)、
> [客户端发布](opencoder-client-deploy-685ec8b09.md)。

## 问一 · 评审范围与原始需求

1. 「按 2 次 tab+shift opencoder 整个会卡住几秒(act clear cotext), 定位并解决， 且不要影响到现有能力.」
2. 「opencoder 触发压缩输出压缩内容就行了， 不要再带上 User: xx 的内容了」
3. 「最新版本的 opencoder 发布到 node02/node05」

验收口径 = 1–2 的静态逻辑闭合 + 定向测试声明绿 + 3 的部署事实(节点版本/SHA256 验证)。

## 问二 · 改动清单(文件:行)

| # | 改动 | 位置 |
|---|------|------|
| 1 | 重建离环核心:`reset_rebuild` 任务槽,`wait_done` 仅在 oneshot resolve 后清槽(取消安全)，内置防死锁单测 | crates/tui/src/reset_rebuild.rs(186 行) |
| 2 | select 双守卫:重建在途禁用 evt 臂/仅启用 wait_done 臂，守卫为假不构造 future,规避双重 &mut 借用 | crates/tui/src/app.rs:735,751 |
| 3 | 重建落位：整视图安装 `chat = rebuilt` + 强制渲染与缓存刷新 | crates/tui/src/app.rs:752 |
| 4 | 会话切换/陈旧防护：switch 成功后才 invalidate;切会话换新 evt 通道，旧队列随旧通道丢弃 | crates/tui/src/app.rs:235、app_task.rs:195 |
| 5 | evt 臂关闭期间不丢事件：无界暂存 + `tx.reserve().await` 背压前向(容量 512) | crates/tui/src/worker/delivery.rs:18-53、worker.rs:79 |
| 6 | fire 路径补设 `pending_turn_echo`(clear-context 场景重建后保留 User 边界) | crates/tui/src/app_loop_actions.rs:468-478 |
| 7 | 压缩去在途回显:`deferred_user_idx` 尾部 trim 匹配跳过；仅 `!compacting` 才尾部重放/补推 | crates/tui/src/session_ui/replay.rs:426,430-439,443-445,482-490 |
| 8 | 压缩分支清 echo+边界重定位；同文本 Compaction 孪生幂等去重(不叠块) | crates/tui/src/session_ui/replay.rs:540,546、crates/tui/src/compaction_block.rs:90-120 |
| 9 | 发布：worktree 固定 HEAD 干净构建 → node02/node05 SHA256 校验+备份+原子 mv | features/changelog/2026-11-17/opencoder-client-deploy-685ec8b09.md:5-15 |
| 10 | 测试与文档：offloop/echo/压缩重放/act_clear 边界用例 + 模块注册 + 前端文档同步 | crates/tui/src/app_loop_tests/transcript_reset_offloop_tests.rs(400 行)、transcript_reset_echo_tests.rs、crates/tui/tests/compaction_summary_replay.rs、crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs、crates/tui/src/lib.rs:46、agents/frontend/tui/index.md:47-53 |

## 问三 · Blocker / 风险

- 代码级 Blocker:无。
- 次要竞态(外观级，本轮新增)：重建安装窗口内非事件臂对 chat 的变更被整视图覆盖 —— poll_bash 输出永久丢失(crates/tui/src/app_notepad.rs:48)、Esc 取消/ArmClearConfirm 标记回退(crates/tui/src/app_loop_actions.rs:643、app.rs:646);subtask tick 每 tick 自愈；旧同步重建无此窗口(crates/tui/src/app.rs:752)。
- 时序缺口(非代码缺陷，记录自洽)：node02/node05 已装 685ec8b09 为**不含未提交 WIP** 的干净构建，即需求 1/2 修复未随本次发布上线(opencoder-client-deploy-685ec8b09.md:5)。

## 问四 · 静态验证(逐项，按问二序号)

1. 逻辑:`wait_done` 仅 resolve 后清槽，drop 重建任务不阻塞循环(内置 `dying_rebuild_task_unblocks_instead_of_deadlocking`)。✔
   影响：同步重建阻塞(卡顿根因)消除，循环持续 tick/渲染；assumptions: offloop 3 例(DelayStore 300ms → fold<150ms/ticks≥5、留队按序、陈旧丢弃)声明通过。
2. 逻辑：两臂互斥守卫使 `reset_rebuild` 单一 &mut 借用成立(编译期保证)。✔
   影响：重建在途 post-reset 事件留队、安装后按序处理；与 4 的 session_id 比对叠加，跨会话重建被丢弃。
3. 逻辑：整视图替换 + `render_pending`/`body_refresh_pending` 同置，同 turn 渲染门重建视口。✔
   影响：即问三竞态窗口本体；running 中提交走队列不动 chat(crates/tui/src/app_submit.rs:80-103),不落入该窗口。
4. 逻辑：invalidate 后置于 switch 成功；换会话即换新通道，旧队列天然丢弃。✔
   影响：跨会话事件不串入新视图；clear-context 的 Reset 后续 AgentSwitch 排队折叠、终态一致(crates/session/src/control_cmd.rs:283-285)。
5. 逻辑：无界暂存 + reserve 前向，满容量仅背压，不丢不乱序。✔
   影响：evt 臂禁用窗口 worker 不丢事件；极端风暴下暂存无上界为理论内存风险，短窗口+512 容量下可忽略。
6. 逻辑:`consumed_echo_text` 命中即推 User 块并记忆 echo,与 `push_user` 同规则。✔
   影响：补上重建后丢 User 边界的缺口；Done/Error 照旧清除(crates/tui/src/chat.rs:354,373);assumptions: act_clear `fired_guard…` 例(含 body_pending+echo+rebuild 边界断言)声明通过。
7. 逻辑:`deferred_user_idx` 取最后一个可见 User 与 echo trim 相等者并跳过；`compacting` 时尾部零重放/补推，会话侧折叠 messages=[summary, tail…](tail 含在途 prompt,crates/session/src/compaction/mod.rs:141-240;Reset+Compaction 事件序 runner/mod.rs:221-222)使匹配成立。✔
   影响：沿用 2026-11-06 trim 匹配口径，若服务端改写尾部文本(trim 不等)则退化为重推记忆 echo、可能与保留副本重复 —— 继承行为、概率低；assumptions: compaction_summary_replay 6 例声明通过。
8. 逻辑:`finalize_compaction` 对同文本非流式孪生幂等去重(重放侧与事件侧均经 sanitize_multiline,文本相等成立，crates/tui/src/chat.rs:285)。✔
   影响：安装后补折的 Compaction 事件不叠块;`pending_turn_echo` 压缩即清，同 turn 再 reset 不复活(replay.rs:540,546)。
9. 逻辑：发布记录内部自洽(worktree 固定 HEAD、SHA256、双备份、原子 mv、节点 --version 验证、运行中进程不受 ETXTBSY 影响)。✔
   影响：该二进制不含 WIP 修复，需求 1/2 生效依赖后续提交+重建+再发布(问五.1)。
10. 逻辑：功能→测试名映射齐备，行数合规(app.rs 800 顶格、app_loop.rs 799、reset_rebuild.rs 186、offloop_tests 400 顶格，均经文件读取核实)。✔
    影响：assumptions: 两 changelog 声明的 TUI 定向测试与 clippy 通过按已声明证据采信；fmt 在 11 个未触及既有文件有漂移、未跑全量测试(符合 rules「改哪测哪」口径)。

## 问五 · TODO / 待确认(收尾前)

1. [必须] 提交当前 WIP → 干净重建 → 按同惯例(SHA256+备份+原子 mv)再发布 node02/node05,使需求 1/2 修复真正上线。
2. [可选] 重建在途窗口对 poll_bash 输出/Esc/ArmClearConfirm 标记加防护(或安装时合并而非整视图覆盖)。
3. [可选] 对 11 个 fmt 漂移的既有文件补一次 `cargo fmt`(与本次逻辑解耦)。

## 上线结论

**go-live ready** —— 三需求逻辑闭合、证据链完整、无未闭环代码级缺陷；唯一余项为 WIP 提交后重建再发布(问五.1),不构成代码阻塞。
