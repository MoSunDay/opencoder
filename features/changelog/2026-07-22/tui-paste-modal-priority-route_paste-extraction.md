# Event::Paste 弹窗优先级链 + `route_paste` 提取至 app_loop.rs

## 背景

`Event::Key` 早已实现弹窗优先级链：当某个模态（task picker / model menu / cache-salt menu / command menu）打开时，按键进入该模态而非主输入框。但 `Event::Paste` 没有对应的优先级链——粘贴的文本**总是**进入主输入框，即使弹窗正打开。这导致在 `/config`、`/model` 等表单中无法粘贴。

把粘贴优先级链内联进 `app.rs` 的 `Event::Paste` arm 会让该文件超过迭代中 800 行上限，因此把整段路由提取为 `app_loop.rs` 的纯函数 `route_paste`（复用既有的 `LoopFlow` 提取模式）。

## 变更

### 弹窗优先级链 + `route_paste` 提取（`crates/tui/src/app_loop.rs`）

新增纯函数 `route_paste(pasted, task_picker_open, cache_salt_menu_open, model_menu, command_menu, input, cursor_idx, workdir) -> LoopFlow`，镜像 `Event::Key` 的优先级：

| 模态状态 | 行为 | 返回 |
|---------|------|------|
| task picker 打开 | 吞掉粘贴（无文本字段） | `LoopFlow::Redraw`（→ `continue` 重绘） |
| cache-salt menu 打开 | 吞掉粘贴 | `LoopFlow::Redraw` |
| model menu 打开 | `ModelMenu::paste(trimmed)` 喂入聚焦字段 | `LoopFlow::Redraw` |
| command menu 打开 | `CommandMenu::paste(trimmed)` 追加查询并 refilter | `LoopFlow::Redraw` |
| 无模态 | 文件路径解析 + 插入主输入（原逻辑） | `LoopFlow::Proceed`（→ 落空，input/cursor 已就地更新） |

`route_paste` 复用了原本标注为「由后续提取块构造」的 `LoopFlow::Redraw`（此前仅有 app.rs 侧的模式匹配、无构造点），顺带清掉了那条 `#[allow(dead_code)]`。同时把 clippy 提示的 `trim_end_matches(|c| c == '\r' || c == '\n')` 改写为 `trim_end_matches(['\r', '\n'])`。

`app.rs` 的 `Event::Paste` arm 瘦身为一次 `route_paste` 调用（`LoopFlow::Redraw` → `continue`）。为补偿工作树中并行任务对 `app.rs` 的新增行，进一步把 `KeyAction::Quit` arm 提取为 `app_loop.rs` 的 `handle_quit` 纯函数（复用同一 `LoopFlow` 模式），`app.rs` 降至 790 行。

### `handle_quit` 提取（`crates/tui/src/app_loop.rs`）

把 `app.rs` 中 `KeyAction::Quit` arm（19 行，含取消令牌 + 推送退出标记 + 发送 `UiCmd::Quit`）提取为 `app_loop.rs` 的 `pub(crate) async fn handle_quit(running, cancel, chat, cmd_tx)`。纯机械搬运，无行为变更；`break` 仍留在 `app.rs` 调用侧。此举把 `app.rs` 保持在 800 行迭代上限以内。

### model_menu 粘贴（`crates/tui/src/model_menu/`）

- `state.rs`：`ModelMenu::paste(&mut self, text)` 按 `Config` / `Form` / `List` 分发；`List` 无文本字段，为 no-op。
- `config_form.rs`：`ConfigForm::paste_into(&mut self, text)` 把数字喂入聚焦的数字字段（`MaxTokens` / `Threshold` / `Fps`），复用 `Char` 分支的逐字段约束：非数字过滤、`threshold` 钳到 ≥1000、`fps` 钳到 1..=30。
- `provider_form.rs`：`ProviderForm::paste_into(&mut self, text)` 按聚焦字段处理——`Name`（只读时跳过）/ `ModelId` / `BaseUrl` 直接追加，`ApiKey` 首次写入时清空并置 `api_key_edited`；当处于 headers 子模式时转交 `HeadersEditor`。
- `headers.rs`：`HeadersEditor::paste_into(&mut self, text)` 把文本追加到当前编辑的 name/value 单元。

### command menu 粘贴（`crates/tui/src/command.rs`）

`CommandMenu::paste(&mut self, text)`：把文本追加到 `query` 并 refilter，镜像 `on_char` 的「push-then-refilter」行为，但一次处理整个字符串。

### 导入清理（`crates/tui/src/app.rs`）

- 移除 `use crate::composer;`（粘贴逻辑已移至 `app_loop.rs`，app.rs 不再直接使用）。
- 从 `pub(crate) use crate::app_helpers::{...}` re-export 中移除 `paste_payload`（现仅由 `app_loop.rs` 直接从 `app_helpers` 导入）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ConfigForm: 数字粘贴进 MaxTokens | `config_form_paste_into_max_tokens` | `model_menu/tests/config_tests.rs` |
| ConfigForm: 过滤非数字 | `config_form_paste_filters_non_digits_in_max_tokens` | `model_menu/tests/config_tests.rs` |
| ConfigForm: fps 钳到 30 | `config_form_paste_into_fps_clamps_at_30` | `model_menu/tests/config_tests.rs` |
| ConfigForm: threshold 前导零拼接 | `config_form_paste_into_threshold` | `model_menu/tests/config_tests.rs` |
| ProviderForm: api_key 首次写入置 edited | `provider_form_paste_into_api_key` | `model_menu/tests/provider_tests.rs` |
| ProviderForm: 追加到 model_id | `provider_form_paste_appends_to_model_id` | `model_menu/tests/provider_tests.rs` |
| ProviderForm: 只读 name 跳过 | `provider_form_paste_skips_readonly_name` | `model_menu/tests/provider_tests.rs` |
| ProviderForm: 追加到 base_url | `provider_form_paste_into_base_url` | `model_menu/tests/provider_tests.rs` |
| ModelMenu::paste 路由到 ProviderForm 字段 | `model_menu_paste_routes_to_provider_form_field` | `model_menu/tests/provider_tests.rs` |
| ModelMenu::paste 在 List 下为 no-op | `model_menu_paste_is_a_noop_in_list` | `model_menu/tests/provider_tests.rs` |
| CommandMenu::paste 追加并 refilter | `paste_appends_to_query_and_refilters` | `command.rs` |
| route_paste: 无模态时插入主输入框 | `route_paste_into_main_composer_inserts_verbatim_text` | `app_loop.rs` |
| route_paste: task picker 打开时吞掉 | `route_paste_swallowed_when_task_picker_open` | `app_loop.rs` |
| route_paste: cache-salt menu 打开时吞掉 | `route_paste_swallowed_when_cache_salt_menu_open` | `app_loop.rs` |

逐字段约束（数字过滤、钳位、只读保护、api_key 掩码）均由对应 `paste_into` 的专门单测覆盖，断言可观测字段值。

- 全量回归（TUI lib）：`cargo test -p opencoder-tui --lib` → 334 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- workspace：`cargo test --workspace` → 全绿（含 store/session/web/cli 各 crate 0 failed）

## Gate

| 项 | 变更前 | 变更后 |
|----|--------|--------|
| app.rs 行数 | 801 行 | 790 行（route_paste + handle_quit 提取后） |
| app_loop.rs 行数 | 384 行 | 535 行 |
| clippy | 0 警告 | 0 警告 |
| TUI lib 测试 | 320 passed | 334 passed（+14，均为本特性粘贴测试（11 model_menu/command + 3 route_paste）） |

> 基线说明：HEAD 处 TUI lib 共 314 个测试属性（`#[test]` + `#[tokio::test]`）。工作树中并行进行的任务（Ctrl+L 提示等）另增数个测试，本粘贴特性净增 14 个（11 model_menu/command 粘贴 + 3 route_paste 路由），合计 `cargo test -p opencoder-tui --lib` 实跑 334 passed / 0 failed。本表「变更前 320」为估算相对基线（334 − 14）。

## Impact Surface

- **向后兼容**：无模态打开时，粘贴行为与原来 100% 一致（文件路径解析 + 插入主输入）。
- **行为变更**：模态打开时粘贴进入模态字段（修复），而非主输入框。
- **不受影响**：session 运行时、store 编码、web 路由、CLI 子命令、LLM 客户端。无 trait / 数据形状 / 公共 API 变更。
