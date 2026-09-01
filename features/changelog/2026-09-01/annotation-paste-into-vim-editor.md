Commit: (working-tree, annotation paste routing + vim paste_terminal)

# /annotation（及 plan/notepad vim 编辑器）括号粘贴路由修复——粘贴不再漏进背后 composer

## Context

用户报障：`/annotation` 应逐字节记录提交内容，但粘贴进来的内容（尤其含 `$skill` 引用的行）会整段丢失。根因在输入路由而非存储链路（`sessions.requirement` 写路径本就逐字节，见 2026-08-15 annotation-raw-roundtrip）：`route_paste`（`crates/tui/src/app_loop_paste.rs`）只认识普通弹窗（task_picker/model/mcp/envs/cli/command/question），不认识全屏 `plan_edit`（/annotation 与 plan 共用）与 notepad。编辑器打开时 `Event::Paste` 一路穿透所有 modal 分支，落到兜底的 `composer::insert_str(input, …)`——粘贴内容被静默写进 overlay 背后的 composer 输入框：编辑器收不到（/annotation 内容从粘贴点起丢失），退出编辑器后 composer 里还残留一份暗文本，回车即误发（`$` 开头还会误激活 skill）。

## Change Summary

- `vim/mod.rs`：新增 `paste_terminal(state, payload)` —— 括号粘贴在 Normal/Insert 模式一律按字面文本插入光标处（vim `paste` 语义，粘贴字节永不解释为命令，`$dd`/`$skill` 不再执行）；经 `composer::insert_str` 走 composer 同款净化与 `MAX_INPUT_CHARS` 上限；Command/Search 模式吞掉（各自有独立输入缓冲）；经 `undo::after_dispatch` 记一步撤销。
- `plan_edit.rs`：`PlanEdit::paste` 适配器，粘贴即标记 modified，`:wq` 正常落库。
- `app_loop_paste.rs`：`route_paste`/`handle_paste_event` 新增 `plan_edit`/`notepad` 参数并置于 modal 优先级最前（镜像 `Event::Key` 的路由顺序）；编辑器打开时空粘贴不再触发背后剪贴板图片读取。
- `app.rs`：`Event::Paste` 臂传入两个编辑器态（文件保持 ≤800 行）。

## Validation

- `cargo test -p opencoder-tui` → 1608 passed / 0 failed（27 套件全绿，含 8 个新测试）
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- 隔离落库树验证（提交树 = HEAD + 本切片 5 文件，独立 worktree + 独立 target）：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告；`cargo test -p opencoder-tui --lib` → **1517 passed / 0 failed**（含本切片 8 个新测试与 4 个既有 route_paste 测试补参版）——共享树 clippy 唯一红点 `chat_sidecar.rs::flatten_sidecar` too_many_arguments（10/7）与本切片零交集，归属并行会话在途文件。
- rules/02 workspace 全量回归：并行会话在共享树活跃构建/清理窗口内无法收敛（ps 实证 `cargo test --workspace --no-fail-fast` / `cargo clean -p` 交替），**复评触发：其 commit 落库后复跑三项目 gate**。

### 测试覆盖

| 测试 | 断言 |
| --- | --- |
| `vim::tests::terminal_paste_insert_mode_appends_literally` | Insert 模式多行粘贴（含 `$token`）逐字追加、光标前进 |
| `vim::tests::terminal_paste_normal_mode_never_executes_vim_commands` | Normal 模式粘贴 `$dd x` 不执行删除，纯字面插入 |
| `vim::tests::terminal_paste_command_mode_is_swallowed` | `:` 命令行模式吞掉粘贴，text/cmdline 不变 |
| `vim::tests::terminal_paste_empty_payload_is_noop` | 空粘贴 no-op |
| `vim::tests::terminal_paste_is_one_undo_step` | 粘贴是一步 `u` 可撤销编辑 |
| `plan_edit::tests::paste_inserts_verbatim_and_marks_modified` | 编辑器适配：逐字插入 + is_modified |
| `image_paste_tests::route_paste_feeds_open_annotation_editor_verbatim` | /annotation 打开时粘贴进编辑器（逐字、modified、Redraw），composer input 保持空 |
| `image_paste_tests::route_paste_swallowed_by_notepad` | notepad Editor 焦点插入 vim 缓冲；Tree 焦点吞掉；composer 两种情况都不收 |
