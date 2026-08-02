Commit: (working-tree, pre-initial-commit)

# TUI 多模态粘贴：data-URI / 图片 URL / ocimg 分段帧重组

## 背景
opencoder 需完备的多模态（vision）能力。远端 SSH/tmux 场景下终端会截断超大粘贴，
无法直接贴图。本次实现 TUI 三条图片摄入路径，统一经纯函数前缀分类（O(前缀)、**不解码
base64**，事件循环永不阻断），让 base64 图片数据作为纯文本流过任意终端。

## 变更
### 分段帧协议 + 纯累加器
- **`crates/tui/src/image_chunk.rs`**（279 行）：`ocimg begin/chunk/end` 自帧定界协议
  解析器（`parse_frame`）、纯字符串拼接累加器 `Assembly`（`feed_line` / `drain_stale` /
  `is_empty`）。累加只做字符串拼接——永不解码 base64，喂帧不会阻塞事件循环。帧顺序无关，
  重复 `seq` 幂等覆盖。`FeedOutcome` 四态：`NotFrame` / `Pending` / `Complete` / `Warn`。
- **`crates/tui/src/image_chunk/tests.rs`**（211 行）：15 个 unit 测试覆盖协议解析、
  乱序拼接、重复覆盖、缺片告警、超时清理、格式映射。
- 粘贴分类纯函数 `image_data_uri_filename`（`data:image/<fmt>;base64,…` → `pasted.<ext>`）
  与 `image_url_filename`（HTTP(S) 图片 URL → 末段文件名）。

### route_paste 集成
- **`crates/tui/src/app_loop.rs:547`**：`route_paste` 新增 `asm: &mut Assembly` 与
  `chat: &mut ChatView` 参数。分类顺序：modal 隔离 → ocimg 帧处理（逐行喂 Assembly，
  `Complete` 推入 `pending_images`，`Warn` 推入 chat 警告行，非帧行落入 composer）
  → `data:image` URI 附图 → 图片 URL 附图 → 本地路径 `try_load_image` → 纯文本插入。
- **`crates/tui/src/app.rs:88`**：声明 `img_asm: Assembly` 状态变量，粘贴时传入。
- **`crates/tui/src/lib.rs:14`**：`pub mod image_chunk;`。
- **`crates/tui/src/app_loop_tests/mod.rs`**：注册 `mod image_paste_tests;`；已有 3 个
  `route_paste` 测试同步更新签名。

### 集成测试
- **`crates/tui/src/app_loop_tests/image_paste_tests.rs`**（325 行）：11 个测试覆盖
  data-URI 附图、图片 URL 附图、非图 URL 插文本、`data:text/plain` 插文本、本地路径
  附图、整段帧一次性完成、跨粘贴增量帧、乱序帧、缺片告警、200KB 长文本不阻断、帧+文本混合。

### 辅助脚本
- **`scripts/img2uri.sh`**（363 行）：本机图片 → data URI / `--chunk KB` 分帧输出，
  可选 ImageMagick 压缩。
- **`scripts/e2e-qwen-vision.sh`**（77 行）：qwen vision 端到端冒烟脚本。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 协议解析 | `parse_frame_valid_variants` / `parse_frame_rejects_malformed` | `image_chunk/tests.rs` |
| 乱序拼接 | `out_of_order_seqs_concat_in_order` | `image_chunk/tests.rs` |
| 重复覆盖 | `duplicate_chunk_overwrites` | `image_chunk/tests.rs` |
| 缺片告警 | `end_with_missing_chunk_warns_and_drops` | `image_chunk/tests.rs` |
| 超时清理 | `drain_stale_drops_old_entries` | `image_chunk/tests.rs` |
| 格式映射 | `jpeg_fmt_maps_mime` / `unknown_fmt_warns` | `image_chunk/tests.rs` |
| data-URI 附图 | `paste_data_uri_attaches_verbatim` | `image_paste_tests.rs` |
| 图片 URL 附图 | `paste_image_url_attaches` | `image_paste_tests.rs` |
| 非图 URL 插文本 | `paste_non_image_url_inserts_text` | `image_paste_tests.rs` |
| text/plain URI 插文本 | `paste_data_text_plain_inserts_text` | `image_paste_tests.rs` |
| 本地路径附图 | `paste_local_path_attaches` | `image_paste_tests.rs` |
| 整段帧完成 | `paste_chunk_block_single_shot` | `image_paste_tests.rs` |
| 增量帧 | `paste_chunk_frames_incremental` | `image_paste_tests.rs` |
| 乱序帧 | `paste_chunk_out_of_order` | `image_paste_tests.rs` |
| 缺片告警 | `paste_chunk_missing_piece_warns` | `image_paste_tests.rs` |
| 长文本不阻断 | `paste_random_long_text_not_blocked` | `image_paste_tests.rs` |
| 帧+文本混合 | `paste_mixed_frames_and_text` | `image_paste_tests.rs` |

- 全量回归：`cargo test --workspace` → 全绿（26 新增测试 + 全量既有测试通过）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`image_chunk.rs` 279 ≤ 400；`image_chunk/tests.rs` 211 ≤ 400；
  `image_paste_tests.rs` 325 ≤ 400；`app_loop.rs` 737 ≤ 800；`app.rs` 800 ≤ 800

## Impact Surface
- 用户：在 TUI 中粘贴 `data:image/...;base64,...` 文本或图片 URL 即可附图；
  超大图可用 `scripts/img2uri.sh --chunk` 分帧粘贴。
- 不影响：CLI / Web / session / store / LLM 后端边界——图片数据以
  `pending_images: Vec<(String,String)>` 的 url 字段（纯字符串）原样透传至下游。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
