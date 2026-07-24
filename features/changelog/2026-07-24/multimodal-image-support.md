Commit: (working-tree, pre-initial-commit)

# feat(core,llm,store,session,cli): multimodal image prompts (vision)

## 背景
此前 OpenCoder 只能处理纯文本对话。模型若支持视觉（vision），用户无法
附带图片让模型分析截图、设计稿或图表。本次贯通「CLI 入参 → core 消息模型
→ store 持久化 → session drain → LLM 请求体」全链路，让
`opencode run "..." --image ./a.png` 能把本地图片作为 `data:` URI 嵌入首条
用户消息发给 vision 模型，并保证 steer/queue/resume 跨 turn 与重启不丢图。

设计约束：纯文本消息保持字节级向后兼容；图片在压缩估算里按固定代价计费
（避免几百 KB 的 base64 撑爆压缩预算）。

## 变更
### 消息模型（core）
- **`crates/core/src/message.rs`**：`ContentBlock` 新增 `Image { url, detail }`
  变体（`url` 为 `http(s)://` 或 `data:image/...;base64,...`，`detail` 映射
  OpenAI `image_url.detail`）；新增 `as_image()`、`has_image()`、
  `user_with_images()`（无图时等价于 `Message::user`）。`text()` 不输出图片
  URL；`estimate_chars()` 对每张图计固定 ~1024 字符（≈256 token）而非展开
  base64（message.rs:191-195），保护压缩阈值。

### LLM 请求体（llm）
- **`crates/llm/src/message.rs`**：`push_user` 走多模态分支——仅当消息含图时
  输出 OpenAI `content` 数组（`text` + `image_url`，可选 `detail`）；纯文本
  消息仍走原 `else` 分支，输出与改动前逐字节一致（向后兼容 + 省 token）。

### 持久化（store）
- **`crates/store/src/types.rs`**：`SessionInput` 新增 `images: Vec<String>`
  （`serde(default, skip_serializing_if = Vec::is_empty)`）。
- **`crates/store/src/libsql_store/schema.rs`**：schema v3→v4，`session_inputs`
  新增 `images_json TEXT NOT NULL DEFAULT '[]'`；`migrate` 在 `from < 4` 时
  `add_column_if_absent`（schema.rs:173-183），旧纯文本行读回为空数组。
- **`crates/store/src/libsql_store/inputs.rs`**：admit/pending/claim_next_queue
  全部读写 `images_json`；`row_to_input*` 反序列化为 `Vec<String>`。

### 会话运行时（session）
- **`crates/session/src/runner.rs`**：新增 `run_with_images()`；`run()` 与
  `run_with_registry()` 增加 `images` 参数，把图作为 `Image` block 附到首条
  用户消息。`claim_steers` / `claim_one_queued` 返回值带上 `images`，drain
  时用 `user_with_images` 重建消息（runner.rs:357-360、418-421），保证 steer
  与 queue 跨 turn 保留图片。
- **`crates/session/src/resume.rs`**：subagent 重放调用 `run_with_registry`
  传入空 images（子代理不继承图片）。
- **`crates/session/src/lib.rs`**：导出 `run_with_images`。

### CLI 入参（cli）
- **`crates/cli/src/lib.rs`**：新增全局 `--image <PATH>`（可重复），置于
  prompt 前以防 trailing prompt 吞掉。
- **`crates/cli/src/run.rs`**：`load_image_data_uris()` 读取文件并 base64 编码
  为 `data:<mime>;base64,...`；`mime_from_ext()` 按扩展名映射 MIME（未知回退
  `image/png`）；缺文件为硬错误（不静默丢弃附件）。有图走 `run_with_images`。

### 适配
- **`crates/tui/src/app_helpers.rs`**、**`crates/web/src/handle.rs`**：
  `mk_input` / `admit_and_drain` 构造 `SessionInput` 时补 `images: Vec::new()`。
- **`Cargo.toml` / `Cargo.lock` / `crates/cli/Cargo.toml`**：引入 `base64 = "0.22"`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| Image 序列化 kind=image | `image_block_serializes_with_image_tag` | core/tests/message_image.rs |
| Image 含/不含 detail 往返 | `image_block_roundtrips_with_and_without_detail` | core/tests/message_image.rs |
| has_image 检测 / text() 排除图 | `has_image_detects_image_blocks` | core/tests/message_image.rs |
| 每个 URI 一个 Image block | `user_with_images_appends_one_image_block_per_uri` | core/tests/message_image.rs |
| 图片固定计费非展开 base64 | `estimate_chars_counts_image_attachment_without_dumping_base64` | core/tests/message_image.rs |
| 旧 blocks JSON 仍可反序列化 | `old_blocks_json_without_image_still_deserializes` | core/tests/message_image.rs |
| 纯文本仍输出字符串 content | `pure_text_user_message_keeps_string_content` | llm/tests/lower_messages.rs |
| 含图降级为 content 数组 | `image_user_message_lowers_to_content_array` | llm/tests/lower_messages.rs |
| detail 字段透传 | `image_detail_is_forwarded_when_present` | llm/tests/lower_messages.rs |
| 图片随 pending_inputs 往返 | `images_roundtrip_through_pending_inputs` | store/tests/images_persistence.rs |
| 图片随 claim_next_queue 往返 | `images_roundtrip_through_claim_next_queue` | store/tests/images_persistence.rs |
| 纯文本 input 图片为空 | `plain_text_input_has_empty_images` | store/tests/images_persistence.rs |
| v3→v4 迁移加 images_json 列 | `schema_v3_to_v4_adds_images_json_column` | store/tests/schema_v4_migration.rs |
| 含图请求落到 image_url | `image_attachment_reaches_request_body` | session/tests/image_request.rs |

- 全量回归：`cargo test --workspace` → 全绿（921 passed / 0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净
- 行数：新文件均 ≤ 400（message_image 90 / image_request 73 / images_persistence 115 / schema_v4_migration 120）

## Impact Surface
- 用户：`opencode run "<prompt>" --image <path>` 可附图给 vision 模型；`--image`
  可重复，缺文件直接报错。无图时行为与之前完全一致。
- 调用方：`run_with_registry` 签名新增 `images` 参数（外部直接调用方需适配）；
  `run`/`run_with_images` 对外 API 不变。`ContentBlock` 新增变体不影响旧 JSON。
- 不影响：store `Store` trait 抽象边界、Web SSE 协议、subagent 调度语义。

## Related Docs
- [agents/core](../../agents/core/index.md)
- [agents/llm](../../agents/llm/index.md)
- [agents/store](../../agents/store/index.md)
- [agents/session](../../agents/session/index.md)
