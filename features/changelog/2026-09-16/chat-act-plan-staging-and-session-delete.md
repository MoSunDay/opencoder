Commit: cbb43c6a

# 会话页 act/plan 前置可选与悬停删除会话

## 交互

- 选中执行节点后，顶部 act/plan 立即可点击，不再要求先有会话。未建会话时点击只在本地暂存模式，创建会话时随 `POST /api/sessions` 的 `agent` 字段一并提交（服务端已有该字段，落 `meta.agent` 并初始化 harness）；已选中会话时空闲走 `POST /api/sessions/:id/agent`，drain 运行中则按 control_cmd 约定以 steer 文本发送 `/act`/`/plan`，且改为直接 POST，不再经过 `send()` 以免清空输入框草稿。斜杠命令 `/act` `/plan` 复用同一分支，未建会话时同样只暂存。
- 顶部工具条只保留 act/plan 与「模型」；批注、压缩、autopilot 按钮移除，对应能力仍经输入框斜杠命令（`/annotation`、`/compact`、`/ap`）与原弹窗可用。
- 会话侧栏条目悬停出现 `...` 菜单（@ant-design/x Conversations `menu`，click 触发），点「删除」后弹 `Modal.confirm` 确认，确认才调 `DELETE /api/sessions/:id`（服务端级联消息/事件/输入并取消运行中 drain）；删除当前会话会清空右侧转写并取消选中，取消确认则不动任何数据。

## Validation

- 新增 `crates/web/spa/src/chat/chatActions.dom.test.jsx`（5 项）：顶栏只留 act/plan+模型、无会话时暂存模式并随创建提交 `agent`、已选会话走 `POST /agent`、悬停菜单删除确认后调用 DELETE 且清空选中、取消确认不删除。
- 更新 `chat/nodeSelection.dom.test.jsx`：创建会话断言补 `agent: 'act'`。
- SPA 全量回归：`npm test`，101 个测试文件、710 项通过。
- `npm run build` + `scripts/check-spa-drift.sh`：无漂移。
- 仅改 SPA 与内嵌产物，Rust 代码零改动（`include_bytes!` 只消费 dist 字节）。

## Related Docs

- [web 模块](../../../agents/web/index.md)

## Release

- 2026-09-16 发布上线:`rel-071c3aca7dd59b767ea4652eed82d7589d03c226`(前序 `rel-92b4ec15`)。发布链在 4a54dfa7 之上补了 dist 重建提交 `071c3aca`(提交的 dist 是旧依赖快照产物,与当前 lockfile 解析的 node_modules 构建结果存在真实漂移;src 未动,710 项 SPA 测试结论延续)。Rust 源码与线上零差异,协议版本 9 不变。
- 验收:`scripts/acceptance/smooth_release/live.py --signal --observe-seconds 900` 结果 PASS(evidence: `/var/tmp/release-live-9164d4c4cd39c6f3`);真实 TODO 链、SSE 恢复 0.15s、持续提交/调度间隔均 < 0.33s,观察满 900 秒;旧 Runtime 进程身份未变。
- 已知问题(非本次引入,已定位):`cargo test --workspace -- --test-threads=4` 下 6 个用例会因进程级 `$HOME` 被并发翻转而偶发失败——`tools::bash::tests::background_output_overflow_stops_process_and_caps_file`(bash 启动脚本打出 dircolors stderr 计入后台文件,越过 `LIMIT+256` 断言余量)、`bash_guard_plan_mode`/`clear_context_bash_gate` 的 `$HOME set: NotPresent`、`shellguard classify_in_tests` 两项同因。串行运行全部通过;根因是这些测试未参与 `test_env.rs::env_lock`,待后续修复(仅测试代码,不阻塞发布)。
