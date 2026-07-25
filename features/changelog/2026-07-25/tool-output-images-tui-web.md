Commit: (working-tree, pre-initial-commit)

# feat(core,llm,session,web,tui): 工具返回图片 + TUI 半块渲染 + web/`/clip` 图片录入

## 背景

前序提交 `9abb9b4` 已落地用户消息的**多模态输入**（`ContentBlock::Image`、
`Message::user_with_images`、store `SessionInput.images` 列、CLI `--image`）。
但图片只进不出——**工具无法把截图/图表交回给模型**，TUI 也看不到图。本轮补齐
三个缺口：

1. **工具输出图片**：`ToolResult` 携带 `images`，转发给 vision 模型为
   `image_url` part；token/压缩按每图固定 ~256 计费，不灌 base64。
2. **TUI 原生图片**：`/clip` 读系统剪贴板、拖拽/粘贴图片路径、transcript 内联
   半块 ASCII 渲染。
3. **web admit 图片**：`POST /sessions/:id/prompt` 接受 `images` 数组。

所有序列化向后兼容：无 `images` 的旧行反序列化为空，纯文本结果字节不变。

## 变更

### core
- **`crates/core/src/message.rs`**：`ContentBlock::ToolResult` 增
  `images: Vec<String>`（`#[serde(default, skip_serializing_if="Vec::is_empty")]`，
  message.rs:33-34）；`estimate_chars` 对工具图片按固定 ~1024 字符计费而非
  base64（message.rs:195-203）。
- **`crates/core/src/tool.rs`**：`ToolOutput` 增 `images`（tool.rs:24-27）+ ctor
  `ok_with_images`（tool.rs:40-47）；`ok`/`err`/`truncate_output_with_error`
  初始化 `images: Vec::new()`。

### llm
- **`crates/llm/src/message.rs`**：新增 `tool_message(id, content, is_error, images)`
  （message.rs:163-181）——有图时 `tool` 消息 `content` 输出为
  `[{type:text},{type:image_url}]` 数组；无图时保持纯字符串（字节不变）。
  `push_user`/`push_tool_results` 均走它。

### session
- **`crates/session/src/tools/image_data.rs`**（新增，112 行）：magic-byte
  `sniff_mime`（png/jpeg/gif/webp/bmp，默认 png）+ `bytes_to_data_uri` +
  `file_to_data_uri`。8 个单测。
- **`crates/session/src/tools/view_image.rs`**（新增，141 行）：`view_image` 工具，
  读本地图片文件 → `ok_with_images`。4 个 `#[tokio::test]`。
- **`crates/session/src/tools/mod.rs`**：注册 `image_data`/`view_image` 模块与
  `ViewImageTool`（mod.rs:14,20,36）。
- **`crates/session/src/runner/mod.rs`**：工具结果装配处把 `out.images` 拷入
  持久化的 `ContentBlock::ToolResult { images: out.images, .. }`（mod.rs:316-321）。

### web
- **`crates/web/src/api.rs`**：`PromptBody` 增 `#[serde(default)] images`（api.rs:100-103），
  `post_prompt` 转发 `images` 到 `admit_and_drain`（api.rs:159）。
- **`crates/web/src/handle.rs`**：`admit_and_drain` 增 `images` 参数，写入
  `SessionInput.images`（handle.rs:111-128）。

### tui
- **`crates/tui/src/image_render.rs`**（新增，159 行）：`decode_data_uri` +
  `render_image_halfblock`（2 行/格、`▀▄`+fg/bg RGB）。7 个单测。
- **`crates/tui/src/image_util.rs`**（新增，162 行）：图片路径探测/加载为 data URI。7 个单测。
- **`crates/tui/src/clipboard.rs`**（新增，55 行）：`clipboard_image_data_uri`（arboard）+
  可测 `encode_rgba_png`。2 个单测。
- **`crates/tui/src/chat_types.rs`**：`ChatBlock::Image { filename, rendered }`（chat_types.rs:39-45）。
- **`crates/tui/src/chat.rs`**：`Image` 块高度记账 + 渲染（chat.rs:378,426,517-536）。
- **`crates/tui/src/session_ui.rs`**：用户 `Image` 块 → 半块渲染入 `ChatBlock::Image`（session_ui.rs:105-130）。
- **`crates/tui/src/app.rs`/`app_loop.rs`/`worker.rs`/`command.rs`/`app_helpers.rs`**：
  `pending_images` 状态、`/clip`/`/cl` 指令、`UiCmd::Prompt(String, Vec<String>)`、
  `mk_input_with_images`、拖拽/粘贴路径探测。

### 依赖
- `Cargo.toml`：workspace `arboard = "3"`；`session`/`tui` 加 `base64`；
  `tui` 加 `image = {0.25, features=["png","jpeg"]}` + `arboard.workspace=true`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| ToolOutput.images 构造/serde/兼容 | `ok_with_images_carries_the_images` 等 5 | core/tests/tool_output_image.rs |
| ToolResult images 序列化/省略/兼容/计费 | `tool_result_with_images_serializes_the_images` 等 5 | core/tests/message_image.rs |
| 工具图降级为 content 数组 | `tool_result_with_image_lowers_to_content_array` 等 5 | llm/tests/lower_messages.rs |
| mime 嗅探/data-uri | `sniff_mime_*`/`bytes_to_data_uri_*` 8 | session/tools/image_data.rs |
| view_image 工具 | `returns_image_inline_for_png` 等 4 | session/tools/view_image.rs |
| web 图片 round-trip | `prompt_body_images_round_trip_to_persisted_message` 等 2 | web/tests/web_image_admit.rs |
| 半块渲染/解码 | `render_rgba_image_pairs` 等 7 | tui/src/image_render.rs |
| 图片路径加载 | `try_load_image_loads_image_file` 等 7 | tui/src/image_util.rs |
| 剪贴板编码 | `encode_rgba_png_roundtrips_as_png_data_uri` 等 2 | tui/src/clipboard.rs |
| mk_input_with_images | `mk_input_with_images_passes_images_through` 等 2 | tui/src/app_helpers_tests.rs |

- 全量回归：`cargo test --workspace`（隔离 target）→ **1057 passed; 0 failed**。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 行数：新增文件均 ≤400（image_render 159、image_util 162、view_image 141、image_data 112、clipboard 55）。

## Impact Surface
- **用户**：工具（如 `view_image`）可把图片交回 vision 模型；TUI `/clip`/拖拽/粘贴录入图片并内联预览；web 可上传图片。
- **向后兼容**：旧消息行无 `images` 反序列化为空；纯文本工具结果字节不变。
- **已知不对称**：TUI 内联渲染目前仅渲染用户 `ContentBlock::Image`；工具 `ToolResult.images` 仍会发给模型并持久化，但 TUI 显示工具的文本输出。

## Related Docs
- [agents/session](../../agents/session/index.md)、[agents/tui](../../agents/tui/index.md)
- 已提交多模态基线：`9abb9b4 feat(core,llm,store,session,cli): multimodal image prompts (vision)`
