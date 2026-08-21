# fix(tui/store): `/task` 切换与 TUI 启动改为纯数据加载，消除 N+1/串行网络/全表扫描

## 背景

- `/task` 切回一个含 Running/Cancelled 子代理的会话时，`switch_session` 在 UI 主循环
  内联 `.await` `resume_and_replay`——它**串行真实重跑**每个未终态子代理（单子代理
  上限 300s + flusher），期间无渲染、无按键响应、取消 token 无人触发，表现为"切走后
  假死/冻结"。
- `draw_resume_replay_banner` 只统计 `Running`，而重放还包括 `Cancelled`：只有
  Cancelled 时无横幅纯假死；有 Running 时先弹全屏横幅再冻结。
- 加载路径多处低效：`reconstruct_child_view` 每个子代理做 2 次全量查询（events +
  messages）；`prefetch_image_bytes` 串行 HTTP 抓图（每图可达 10s）；`sessions::list`
  的 preview 子查询拉整块 `blocks_json`（首条 user 消息含 base64 图时可达 MB 级）；
  `subagent_tasks.task_id` 无索引（COMPLETE/CANCEL/get_by_task_id 全表扫）。

## 变更

### TUI 切换快路径（`crates/tui/src/app_task.rs`）

- Resume/Fork 两臂改走新助手 `load_session_for_switch(...) -> (SessionState, usize)`：
  只调 `opencoder_session::resume::resume`（纯数据加载，不触发 LLM 重放），并用一次有
  索引的 `list_subagent_tasks` 统计 pending（Running|Cancelled）数。
- **删除** banner 三件套（`draw_resume_replay_banner` / `render_resume_replay_banner` /
  `resume_banner_message`）及 `BannerStore` 测试桩、`switch_session` 的 `terminal` 参数。
- pending>0 时改为 `chat.push_marker("[task] N subagent(s) replay pending — resume on
  next message")`（纯函数 `pending_replay_hint(n)`），与 `/task` picker 的
  `⊗ N replay pending` 徽标呼应；重放由下一轮 prompt 时既有的
  `replay_cancelled_tasks` 承接（去重/边界守卫/steer 时放弃/Esc 可取消/spinner）。
- `cmd_tx.send(UiCmd::Quit).await` → `try_send`：通道满（worker 忙）时不再挂住 UI
  事件循环；旧 worker 反正随 sender drop / `rebind_session` 换绑退出。

### TUI 启动快路径（`crates/tui/src/app_bootstrap.rs`）

- `-s/--session` 启动加载同样由 `resume_and_replay` 改为 `resume`（同一症状：启动不
  该被重放阻塞）；CLI headless（`cli/run.rs`）与 web（`web/handle.rs`）保持
  `resume_and_replay`——它们重放后立即 prompt，语义等价。

### 重建提速（`crates/tui/src/session_ui/replay.rs`）

- `reconstruct_child_view`：events 命中分支删除第二次全量 `load_messages`，
  `context_used` 直接用 `view.apply` 在事件重放中已累计的值（崩溃丢失事件日志尾部时
  仅轻微低估，显示启发式）；messages fallback 分支不变。
- `prefetch_image_bytes`：改并发（`JoinSet`）+ 整体预算 8s（`PREFETCH_BUDGET`，超时
  abort 剩余、部分成功即可）；fetcher 以参数注入（
  `prefetch_image_bytes_with(messages, fetch, budget)`）保持可测。

### Store 提速（`crates/store/src/libsql_store/`）

- `schema.rs`：随主索引批次新增 `CREATE INDEX IF NOT EXISTS idx_subagent_task_id ON
  subagent_tasks(task_id)`（幂等；task_id 列自建表起存在，无需版本号变更）。
- `sessions.rs::list`：preview 子查询改 `substr(m.blocks_json, 1, 8192)`；
  `extract_preview` 解析失败（截断 JSON）→ 空 preview 安全降级。

### 语义变化（有意为之）

- pending 子代理从"切换时同步重放完成"改为"下一条消息时重放"；有 steer/queue 在场时
  按既有守卫放弃。TUI `-s` 启动同样变为"不重放"（与 `/task` 一致）。
- 代价（纯外观）：首条 user 消息图片优先或文本超 8KB 时 `/task` 列表 preview 变空。

### 附带（工作树内既有的 todos 半成品收尾，非本主题）

- 完成 `transitions::candidate` 新 `spec` 参数在 `batch.rs` / `transitions.rs` 测试的
  调用点；`recovery.rs::acceptance_crash_then_resume_self_heals` 的挂起态断言按新
  语义（Suspended 回滚 mid-flight 为 Interrupted）更新；移除 `interrupt_retry.rs`
  未用 import。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 切换纯加载不重放（Cancelled 保持、无合成回填、子会话不动、pending 计数） | `load_session_for_switch_is_pure_load_no_replay` | `crates/tui/src/app_task.rs` |
| pending 只计 Running+Cancelled | `load_session_for_switch_counts_only_pending_statuses` | `crates/tui/src/app_task.rs` |
| 切换提示 marker 文案 | `pending_replay_hint_none_for_zero` / `pending_replay_hint_lists_count_and_trigger` | `crates/tui/src/app_task.rs` |
| prefetch 并发 | `prefetch_fetches_run_concurrently` | `crates/tui/src/session_ui/image_prefetch_tests.rs` |
| prefetch 预算（部分成功+总时长有界） | `prefetch_budget_returns_partial_success_and_bounds_wait` | `crates/tui/src/session_ui/image_prefetch_tests.rs` |
| prefetch 空 URL 集零开销 | `prefetch_empty_url_set_is_free` | `crates/tui/src/session_ui/image_prefetch_tests.rs` |
| 新索引存在且幂等 | `bootstrap_creates_subagent_task_id_index` | `crates/store/tests/store_migrations.rs` |
| preview 截断（cap 内可提取 / 图片优先截断降级 / 超长文本降级） | `preview_extracts_text_within_cap` / `preview_degrades_when_image_first_overflows_cap` / `preview_degrades_when_text_exceeds_cap` | `crates/store/tests/preview_truncate.rs` |
| 回归：切换后 queue/steer 镜像 | `resume_restores_display_originals_and_drain_stays_clean` | `crates/tui/tests/resume_queue_display.rs` |
| 回归：`resume_and_replay` 本体不动 | `resume_replay*` / `resume_cancelled_pending` 全套 | `crates/session/tests/` |

- 全量回归：`cargo test --workspace` → 全部通过（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
