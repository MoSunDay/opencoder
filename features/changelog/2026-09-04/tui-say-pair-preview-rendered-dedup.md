Commit: (working-tree, 待提交)

# TUI 合并头 `Say(n step):` preview 改渲染口径：修「头部 markdown 不渲染 + 头部下方复述正文首行」

## Context

用户报告：turn 最后一个 step 要启动 subagent 时，`Say(n step): xxx` 头部的
xxx 内容 markdown 不渲染；且概率性在启动 subagent 前，于头部下方区域把
xxx 内容再复述一遍。

根因（单点、两症状同源）：合并对头部的 preview 固定取 **raw** 首行
（`say_preview`），而 done 正文的去重比较的是 **markdown 渲染后** 的首行
文本（`merged_say_body` 的 trim 相等口径）。首行含 markdown（`#`/`**`/
`-` 等）时两者永不相等：

1. 头部 preview 露出**原始 markdown 标记**（`Say(2 steps): **派个 subagent**
   去查`）——「markdown 不渲染」；
2. `merged_say_body` 判 Full，正文把首行**以渲染形态再输出一遍**——
   「复述 xxx」；首行纯文本时渲染=raw，比较恰好相等被跳过——故表现为
   「概率性」（实为首行是否含 markdown）。

subagent 派发场景（`ToolStart(task)` 不触发 Say finalize、`SubagentStart`
才收口）使该合并对长时间停留在屏上，最容易观测到；普通工具轮同样存在。
此前 `say_pair_dedup.rs::merged_pair_keeps_full_body_when_first_line_differs_
from_preview` 恰好把该行为钉死为契约，本轮纠正该语义模型。

## Change Summary

- **`chat_step_render.rs`**：新增 `rendered_preview`（done 渲染结果首个
  非空行文本，与 `merged_say_body` 逐行 `line_text` 同口径）与
  `say_preview_for(raw, rendered, done)`——done 取渲染首行（渲染为空回退
  raw 防头部无 preview），流式取 raw 首行（与流式正文行同源）。
  `merged_say_body_decision` 与 `SayHeader` 头部共用这一个口径；
  `SayHeader{raw, streaming}` → `SayHeader{preview, streaming}`（preview
  由构造点一次算好）。
- **`chat_flatten.rs`**：SayHeader 构造点解构 `rendered`，传入
  `say_preview_for` 计算 preview。
- 效果：done 头部显示渲染后文本（无原始标记）；去重比较渲染对渲染，
  首行恒 trim 相等 → Skip/Hidden，头部下方不再复述首行；单行 markdown
  Say 整块隐藏（与单行纯文本 Say 一致）。流式窗口（raw 正文 + raw
  preview）行为不变。行数记账（`chat_headers.rs`/`line_accounting.rs`
  均委托 `merged_say_body_decision`）与 copy 模式（按前缀/标签形状匹
  配，不匹配 preview 内容）自动同步，无需改动。
- 顺带清偿存量 clippy（`-D warnings` 红灯）：`chat_headers.rs` /
  `line_accounting.rs` 的 `nonminimal_bool` 化简、`worker/tests.rs` 的
  `needless_borrows_for_generic_args`（并发会话在途文件，仅机械去
  `&`，无语义变化）；`cargo fmt -p opencoder-tui` 顺带规范化并发会话
  在途测试的换行。

## Impact Surface

- 仅 `crates/tui` 渲染层：`chat_step_render.rs` / `chat_flatten.rs`；
  `chat_headers.rs`、`chat_tests/line_accounting.rs`、`worker/tests.rs`
  仅 lint/格式。无事件协议、Store、session 变更；SPA 不涉及（其正文本就
  无 markdown 渲染，且 say/preview 逻辑独立）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 派发对头部渲染 preview + 正文不复述首行（用户症状 a+b） | `subagent_dispatch_header_renders_markdown_and_body_does_not_repeat` | `crates/tui/src/chat_tests/subagent_dispatch_say.rs` |
| 流式窗口 raw 正文首行跳过、无复述 | `subagent_dispatch_live_window_streams_raw_without_duplication` | 同上 |
| 单行 markdown Say 整块隐藏（头部即全部） | `subagent_dispatch_single_line_markdown_say_hides_body` | 同上 |
| markdown 首行：头部显渲染文本、正文跳过该行（新契约，替换旧「保持 Full」契约） | `merged_pair_renders_markdown_preview_and_skips_it_in_body` | `crates/tui/src/chat_tests/say_pair_dedup.rs` |
| 纯文本首行 Skip / 单行整块隐藏（原 `merged_pair_keeps_full_body_when_first_rendered_line_differs` 更名，名实对齐） | `plain_first_line_still_skips_in_body` | 同上 |
| Full 分支直接单元测试（preview≠rows，防御性：经管线已结构性不可达） | `merged_say_body_full_when_preview_differs_from_every_row` | 同上 |

## 回归

- `cargo test -p opencoder-tui` → **1782 passed / 0 failed**（P2 整改后
  1779→1782 共 +3：我方 Full 分支直接单测 +1，并发会话新增 +2）
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- `cargo fmt -p opencoder-tui` → 已格式化
- **workspace 三 gate 全绿（2026-09-05 04:14–04:42，P1 回填）**：
  - `cargo build --workspace` → ✅ exit 0
  - `cargo clippy --workspace --all-targets -- -D warnings` → ✅ exit 0
    （全 workspace 零警告）
  - `cargo test --workspace` → ✅ exit 0（299 个 `test result:` 全 ok、
    0 failed，其中 opencoder-tui lib 套件 1692/0）
  - 留痕：03:50 与 03:57 两轮先行尝试曾红，唯一根因为并发会话在途未跟踪
    文件 `crates/web/src/api_agent_nfs.rs` 的 `opencode_core` 未解析 import
    （拼写，04:14 由对方修复）；其余 crate（含 opencoder-tui）在所有轮次
    均编译通过，红果与本轮 tui 变更无关
