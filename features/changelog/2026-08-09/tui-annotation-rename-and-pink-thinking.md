# refactor(tui): /requirement 斜杠命令更名为 /annotation；Thinking 标签改用粉色

## 背景

1. `/requirement`（别名 `/req`）斜杠命令编辑的是任务描述备注，"requirement" 偏重，`/annotation`
   （别名 `/ann`）更贴切。
2. `ChatBlock::Thinking`（推理摘要）此前与 bash 工具头共用 `theme::accent()`（Cyan + BOLD），
   视觉上无法与 bash 工具块区分。

## 变更

- **斜杠命令更名（纯表现层 rename，TUI crate）**：`/requirement` → `/annotation`，别名 `/req` → `/ann`。
  - `command.rs`：`COMMANDS` 条目、`SlashAction::Requirement` → `SlashAction::Annotation`（含枚举定义）、
    `parse`（`"annotation" | "ann"`）、`dispatch`、3 个单测更名。
  - `plan_edit.rs`：`EditKind::Requirement` → `EditKind::Annotation`（含枚举定义）、`new_requirement` →
    `new_annotation`、`enter_requirement` → `enter_annotation`、标题 `"edit requirement"` → `"edit annotation"`、
    `border_color` 分支与 2 个单测更名。
  - `chat_types.rs` / `chat_req.rs`：`requirement_text` → `annotation_text`、
    `last_requirement_text` → `last_annotation_text`、`update_requirement_text` → `update_annotation_text`
    及 3 个单测更名。
  - `worker.rs`：`UiCmd::EditRequirement` → `UiCmd::EditAnnotation`（落盘仍写 `SessionPatch.requirement`）。
  - `app.rs` / `app_loop.rs`：拦截串 `/requirement` → `/annotation`、`mode_flash` `→ requirement` → `→ annotation`、
    `TranscriptReset` 的 `saved_*` 局部同步更名。
  - `render.rs`：`is_requirement` → `is_annotation`、`edit_title == "edit requirement"` → `"edit annotation"`
    （composer 边框绿着色 + 顶边镜像 info 标题逻辑不变）。
- **持久化层不动**：DB 列 `sessions.requirement`（schema v8）、`SessionMeta.requirement`、
  `SessionPatch.requirement`、`sess.requirement` 均为内部存储名，保留不变（更名列风险高且超范围）；
  `note_requirement_submitted`（plan 模式用户需求计数，独立概念）亦保留。
- **Thinking 标签改色**：`theme.rs` 新增语义色槽 `pink`（`Palette.pink` + `pub const PINK` + `pub fn pink()`，
  dark=`Color::LightMagenta`、light=`Color::Magenta`，与 LOCAL 的 `Magenta` 区分）；
  `chat.rs` Thinking 块 header 由 `theme::accent()` 改为 `theme::pink()`（仍 + BOLD，body 仍 muted）。
  bash 工具头保持 Cyan accent 不变，二者从此视觉分离。
- `compaction_block.rs` doc、`theme.rs`/`chat_types.rs` 等注释同步更新。

## 测试清单（crates/tui，全部 unit，TestBackend / 纯函数）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| `/annotation` 与 `/ann` 解析、`/annotation` dispatch | `parse_annotation_full` / `parse_annotation_alias` / `dispatch_annotation` | unit(command) |
| `new_annotation` 初始 NORMAL + `EditKind::Annotation` + 标题 `edit annotation` | `new_annotation_starts_normal_with_annotation_kind` | unit(plan_edit) |
| `enter_annotation` 种入编辑器并保留文本 | `enter_annotation_sets_editor` | unit(plan_edit) |
| `annotation_text` 优先/回退/空回退/净化 | `returns_explicit_annotation_when_set` / `falls_back_to_first_prompt` / `returns_none_when_nothing_set` / `empty_annotation_falls_back_to_first_prompt` / `update_annotation_text_sanitizes` | unit(chat_req) |
| `/annotation` 顶边含左侧 `edit annotation` 与右侧 model；model cell 像素级 `fg == Some(green)` | `annotation_editor_shows_green_top_title` | unit(render) |
| Thinking 块折叠/展开/行计数渲染 | `thinking_block_collapses` / `thinking_header_shows_line_count_when_collapsed` | unit(chat/thinking_state) |

## Gate

`cargo build --workspace` 通过；`cargo test -p opencoder-tui`：1098 passed / 0 failed。
