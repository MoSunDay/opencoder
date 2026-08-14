Commit: 625e2ea, 5686b15

# 17-bug 审计计划落地：14 项修复 + 1 项非 bug + 2 项顺带 clippy（#13 按计划 deferred）

## 背景
对 workspace 全量只读审查产出 17-bug 修复计划（阶段 1-4）。本轮执行其中 14 项修复（#5 审查确认为非 bug——display.rs 的 ESC 字节已存在；#13 悬空事务按计划 deferred），每项附回归测试。

## 变更

### plan 模式 bash 写拦截逃逸（#1/#2）
- **`crates/session/src/bash_guard.rs`**：新增 `extract_command_substitutions()` + `find_matching_paren()`——分类前递归提取 `$()`/反引号/`<()`/`>()` 内层命令逐段分类；`classify_segment` 在 `strip_wrappers` 剥壳后重查 `eval|source|.`，堵住 `bash -c 'eval rm …'` 二次逃逸。
- clippy 顺带：`find_matching_paren` 改 `enumerate().skip()` 消 `needless_range_loop`。

### web 会话句柄生命周期（#3/#4）
- **`crates/web/src/handle.rs`**：drain 完成路径 drop 顺序修正为 `drop(sink)` → `drop(rx_guard)` → `drop(guard)` → `flusher.await`——先释放订阅者计数再清 draining 标志，避免早订阅者短暂观察到「无句柄且无 drain」窗口。`release_events_subscriber` 两分支删除 `created &&` 前置——无论句柄是否本连接创建，离开时都要递减订阅计数，防泄漏阻止 drain 重 spawn。

### 配置合并（#6）
- **`crates/core/src/config/merge.rs`**：顶级 `provider` 块合并补齐 `model` 字段与 `headers`（extend 追加，不覆盖已有键）。

### web 路由与过滤（#7/#8）
- **`crates/web/src/lib.rs`**：`build_app` 新增 `web: bool` 参数，条件挂载 `/` HTML 路由（纯 API 模式不再暴露页面）。调用点 `tests/auth.rs`、`tests/client_e2e.rs` 同步更新。
- **`crates/web/src/api.rs`**：events 过滤的 `baseline` 改 `Option<i64>` + `is_some_and`，消除 `baseline == 0` 时的语义歧义。

### LLM 客户端流解析（#9-#12）
- **`crates/llm/src/client.rs`**：`extract_reasoning(delta)` 提到 if/else 之前（此前 thinking 块内 text 通道的 reasoning 被丢弃）；text 块缺 `text` 字段时 `.or_else(content)` 回退；thinking 臂加 `if !emitted_reasoning` 守卫防重复；`get_tokens` 返回 `Option<u64>`（`as_u64` 失败回退 `as_f64`，覆盖 GLM 等返回浮点 usage 的后端——prompt/completion/total/cached_tokens/first_u64 全路径）；429 Retry-After 解析失败回退 HTTP-date（新模块 `src/http_date.rs`，RFC 7231 GMT 解析）。

### store 契约加固（#14/#15/#16）
- **`crates/store/src/libsql_store/messages.rs`**：`append_many` 补 Non-atomic 文档注释（多 batch 间无事务包裹，崩溃可留部分写入——现状记录，非行为变更）。
- **`crates/store/src/libsql_store/sessions.rs`**：`update()` 加 6 组互斥校验——summary/summary_seq/summary_images↔clear_summary、handoff_plan/handoff_seq↔clear_handoff、skill↔clear_skill，同 patch 同时设字段与清标志即 `Err`。
- **`crates/store/src/libsql_store/subagent_tasks.rs`**：`complete`/`cancel` 检查 rows==0 时 `bail!("subagent_task not found")`——late-complete 幂等覆盖改为显式报错（终态不可覆写）；对应用例断言改 `is_err()`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| #1 `$(...)` 写命令拦截 | `bash_guard_blocks_command_substitution_with_write` | session/src/bash_guard_tests.rs |
| #1 反引号替换拦截 | `bash_guard_blocks_backtick_substitution_with_write` | session/src/bash_guard_tests.rs |
| #1 `<()` 进程替换拦截 | `bash_guard_blocks_process_substitution_with_write` | session/src/bash_guard_tests.rs |
| #1 只读替换放行（无过杀） | `bash_guard_allows_command_substitution_with_readonly` | session/src/bash_guard_tests.rs |
| #1 复合命令嵌套替换 | `bash_guard_blocks_nested_substitution_in_compound` | session/src/bash_guard_tests.rs |
| #2 剥壳后 eval 拦截 | `bash_guard_blocks_wrapped_eval` | session/src/bash_guard_tests.rs |
| #2 剥壳后 source 拦截 | `bash_guard_blocks_wrapped_source` | session/src/bash_guard_tests.rs |
| #2 剥壳后 `.` source 拦截 | `bash_guard_blocks_wrapped_dot_source` | session/src/bash_guard_tests.rs |
| #3 drop 顺序：先还 rx 再清 draining | `drain_completion_restores_cmd_rx_before_clearing_draining` | web/tests/handle_bugfix.rs |
| #6 顶级 provider model/headers 合并 | `merge_top_level_provider_model_and_headers` | core/src/config/merge.rs |
| #7 web=false 不挂 HTML 路由 | `web_disabled_omits_html_route` | web/src/lib.rs |
| #7 web=true 挂 HTML 路由 | `web_enabled_serves_html_route` | web/src/lib.rs |
| #9 text 块 content 回退 | `emit_delta_text_block_uses_content_fallback` | llm/src/client_tests.rs |
| #10 浮点 usage 解析 | `parse_usage_handles_float_tokens` | llm/src/client_tests.rs |
| #12 HTTP-date 解析（正常） | `parse_http_date_to_secs_parses_rfc7231` | llm/src/client_tests.rs |
| #12 HTTP-date 解析（非 GMT 拒绝） | `parse_http_date_to_secs_rejects_non_gmt` | llm/src/client_tests.rs |
| #12 HTTP-date 解析（未来为正） | `parse_http_date_to_secs_future_date_is_positive` | llm/src/client_tests.rs |
| #15 字段+clear 混合被拒 | `field_and_clear_combinations_are_rejected` | store/tests/session_patch_conflict.rs |
| #15 无关字段与 clear 共存放行 | `unrelated_field_and_clear_still_succeeds` | store/tests/session_patch_conflict.rs |
| #15 仅 clear 标志放行 | `clear_flag_alone_succeeds` | store/tests/session_patch_conflict.rs |
| #14 多 batch append 全量落盘 | `multi_batch_append_persists_all_in_order` | store/tests/append_many_chunking.rs |
| #14 单消息 append 单 seq | `single_message_append_returns_one_seq` | store/tests/append_many_chunking.rs |
| #16 late-complete 报错（改断言） | `complete_does_not_overwrite_completed_terminal_state` | store/src/libsql_store/subagent_tasks.rs |

- 全量回归（当次实跑，`cargo test --workspace`）：**2464 passed / 0 failed**（152 套件全 ok）。分 crate：core 169（08-13 基线 167）/ llm 124（+5，恰为本轮新测试）/ store 93（+5，恰为本轮新测试）/ web 85（81 基线 +3 本轮）/ cli 91（持平）/ client 9（持平）/ session、tui 无下降。无任何 crate 低于上轮记录。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- e2e: skipped（无 API key；本轮变更未触及 prompt 契约/跨进程恢复深度面，集成层已覆盖）

## Impact Surface
- **用户可见**：plan 模式无法再经命令替换/剥壳 eval 逃逸写盘；GLM 类浮点 usage 后端 token 计数不再丢 0；`--no-web` 类纯 API 模式不暴露 HTML；session patch 字段/清除冲突显式报错而非静默后写覆盖
- **不影响**：`Store` trait 签名、`ChatStream` trait、drain 主循环语义、DB schema（无迁移）
- **已知遗留**：#13（悬空事务）按计划 deferred；`append_many` 跨 batch 非原子性以文档记录，待后续事务包裹

## Related Docs
- [agents/session](../../agents/session/index.md) — bash_guard 命令替换语义（本轮已更新）
- [agents/store](../../agents/store/index.md) — update_session 互斥校验（本轮已更新）
