# TUI 回合失败只上报一条 Error 事件

一句话：本地回合失败时，worker 用 `Arc<AtomicBool>` 记录 runner 是否已自行发出
`SessionEvent::Error`，仅在 `Err` 没有对应错误事件时补发兜底 Error，使 UI 与
事件日志严格只出现一条错误（与 web 侧 `run_emitted_error` 对齐）。

## 背景

runner 在致命失败时会先发 `SessionEvent::Error` 再返回 `Err`；`process_cmd` 的
兜底分支又在 `Err` 时合成一条 Error，导致同一失败回合显示/持久化两条错误。

## 变更

- 每个 `UiCmd::Prompt` 新建 per-turn 的 `Arc<AtomicBool>`，不跨回合复用。
- `run_session` / `run_with_images` 回调看到 `SessionEvent::Error` 即置位；
  仅当 `Err` 分支读到的标志为 false 时才 `sink.push` + `forward_event` 兜底。
- `Relaxed` 足够（同一 task 内、隔一次 await）；`Compact` 分支不变。
- 已知窄边界：若先出现非致命 Error、随后走静默 `Err`，终态错误会被抑制；
  与 web 现有语义一致，属“恰好一条”的取舍。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 失败回合只转发一条 Error | `prompt_failure_error_is_forwarded_exactly_once` | `crates/tui/src/worker/tests.rs` |

- 测试范围：用户已在提交前完成定向测试；本次按指示免测提交，未重复运行。
- 未运行全量测试。
