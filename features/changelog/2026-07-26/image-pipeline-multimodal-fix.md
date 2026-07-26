Commit: (working-tree)

# fix(image-pipeline,llm,session,tui): 工具/用户图片在完整生命周期内送达视觉模型

## 背景

视觉模型（qwen-vl 等 OpenAI 兼容 endpoint）按 Chat Completions 规范接收图片，
但端到端有 4 个根因让图片永远无法以合规请求体抵达模型：

- **RC1（llm/message）**：工具返回的图片被塞进 `role:"tool"` 消息的 `content`
  数组。OpenAI 规范要求 tool 消息的 content 必须是纯字符串——违规体被 provider
  拒绝或静默丢弃图片。
- **RC2（tui/app）**：纯 skill 提交（Submit/Steer/Queue 三条分支）与多个输入入口
  要么静默丢弃 `pending_images`，要么让图片泄漏到后续不相关的提交。
- **RC3（session/runner）**：空用户文本直接跳过整轮（含图片），导致「仅图片无文本」
  的提交被完全吞掉。
- **RC4（compaction/resume/handoff）**：摘要永久丢弃所有图片——压缩后 head 图片
  消失，恢复/handoff 重建的合成消息也不带图片。

## 变更

### RC1 — `crates/llm/src/message.rs`（lowering 输出形状）
- `tool_message()` 现在**永远返回字符串 content**（合规），不再把图片块放进 tool 消息。
- 新增 `tool_image_user_message()`：把工具返回的图片重安置到一条尾随的
  `role:"user"` 多模态消息（合法 `image_url` 块）。`push_tool_results()` 与
  `push_user()` 均在检测到图片时 emit 该重安置消息。
- `mk_input` 旧包装删除（已被 `mk_input_with_images` 取代），统一走带图片入参。

### RC2 — `crates/tui/src/app_helpers.rs` + `app.rs`（图片排空）
- 新增 `drain_pending_images(&mut Vec<(String,String)>) -> Vec<String>`：一步把
  pending 图片排空为 URI 向量并清空缓冲。所有 7 个提交入口（Submit/pure-skill/
  Steer×2/Queue×2/start_turn）统一套用排空模式，杜绝静默丢弃或跨提交泄漏。

### RC3 — `crates/session/src/runner/mod.rs`（空文本守卫）
- 空用户文本的跳过守卫拓宽为「同时检查非空图片」：空文本但有图片时不再跳过整轮，
  图片随 turn 正常记录与发送。

### RC4 — `crates/session/src/compaction.rs` + `resume.rs` + `plan_handoff.rs`（图片保留契约）
- 新增 `collect_head_images(head) -> Vec<String>`：收集 head 中用户/工具图片 URL，
  上限 `MAX_PRESERVED_IMAGES=4`，取最近 4 张。
- 新增 `strip_images(messages)`：把所有图片块从送入 summarizer 的输入中剥离（避免
  图片被摘要吞掉/计费，且降低摘要输入噪声）。
- `compact()` 先 strip 再 summarize，最后把保留的 ≤4 张图片作为 `image_url` 块
  附加到合成 summary 消息（`has_image()` 为真）。
- `resume()`（handoff 与 compaction 两条分支）与 live `plan_handoff` 对重建的合成
  消息同样 re-derive head 图片并附加——跨进程恢复后图片不丢。

### 结构性清理 — `app.rs` 拆分（迭代文件行数合规）
- `crates/tui/src/app.rs` 此前 825 行（超 800 行迭代上限，本次 +23 加重）。把终端
  resize 处理（`size_changed` / `on_resize_event` / `poll_idle_resize`）与 store 初始化
  （`open_store`）抽到既有的「抽出辅助函数」文件 `app_helpers.rs`（该文件 doc 明确
  声明其职责即「keep app.rs under the 800-line iteration cap」）。纯行为不变的重构：
  `app.rs` → **796 行**，`app_helpers.rs` → **793 行**，二者均合规。

## 测试覆盖

新增 **17** 个测试函数，删除 **5** 个**编码了旧违规行为**的过时断言（它们断言「图片
存在于 role:tool content 数组」——正是被修复的 spec 违规），**净 +12**。被删除的测试
由断言新合规形状的等价测试取代。

| RC | 文件 | 新增测试 | 删除（旧违规断言） |
|----|------|---------|--------------------|
| RC1 | `crates/llm/tests/lower_messages.rs` | 5（tool 图片重安置到 user 消息；多图全重安置；error tool 结果图片带前缀字符串重安置；tool content 恒为字符串；user 内嵌 tool 结果图片重安置） | 4（旧：断言 tool 图片在 content 数组） |
| RC1 | `crates/session/tests/tool_image_contract.rs` | 0（1 个既有契约测试改写：断言 tool content 为字符串 + 图片重安置到 `role:"user"` 的 `image_url` 块，HTTP 请求体级） | 0 |
| RC2 | `crates/tui/src/app_helpers_tests.rs` | 3（drain 收集并清空；drain 空返回空；mk_input_with_images 无图时默认空） | 1（旧 `mk_input_without_images_defaults_empty`） |
| RC3 | `crates/session/tests/image_request.rs` | 1（空文本 + 图片仍被记录并发送，断言请求体含 `image_url`） | 0 |
| RC4 | `crates/session/src/compaction.rs`（unit） | 5（collect 收集 user+tool 图；cap@MAX 取最近；空 head 为空；strip 去图留文；strip 清 tool 结果图） | 0 |
| RC4 | `crates/session/src/compaction.rs`（integ） | 1（压缩后 summary 消息 `has_image()` 且 ≤MAX，MockChatClient） | 0 |
| RC4 | `crates/session/tests/resume_image_survival.rs`（integ） | 2（resume-after-compaction 保留 head 图；resume-after-handoff 保留 head 图，真 Store 断言具体 URL） | 0 |
| **合计** | | **17** | **5 → 净 +12** |

测试分层（合规）：unit 测试在源文件 `#[cfg(test)] mod`（零 I/O、MockChatClient、<10ms），
遵循既有 `compact_honors_cancel` 约定；integration 测试在 `tests/`（`LibsqlStore::open_memory`
+ MockChatClient，无真 LLM/DB）。所有 12 个净新增测试断言可观察输出（HTTP 请求体的
`image_url` JSON、重建 transcript 结构 + 具体 URL、ordering/cap 的精确 `assert_eq!`），
无 `is_ok/is_some/assert!(true)` 弱断言。

## 验证（实跑）

- `cargo test --workspace` → **全绿，0 failed**（image-pipeline 全部 17 新增 + 改写契约
  测试逐一直跑通过；workspace 总数相对父提交基线净 +12）。
- `cargo clippy --workspace --all-targets -- -D warnings` → **0 warning**。
- `cargo build --workspace` → **Finished** clean。
- 防修绿 diff 扫描：0 新 `#[ignore]`、0 删 `#[test]`（删除的是编码旧违规的断言，已被
  等价新测试取代）、0 弱断言、0 调试输出（`println!/dbg!/todo!`）、0 硬编码密钥。

## 注意

工作树中存在**范围外**的并行改动（另一会话编辑 `bash.rs`、`app_loop_tests.rs`、
未追踪的 `app_loop_plan_edit_tests.rs` 等），已正确排除——本变更仅触及图片管线相关文件
+ 本次结构性清理（`app.rs` / `app_helpers.rs` / `app_tests.rs` 的 resize/store 抽取）。
