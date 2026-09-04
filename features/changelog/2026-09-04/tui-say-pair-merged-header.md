Commit: (working-tree, 待提交)

# TUI `n Steps + Say` 相邻对合并：单行 `Say(n step{s}): <预览>` 头

## Context

零点击视图里每个工具 Turn 渲染两行 chrome：`▸ N Steps`（Turn 行）与
`❯ Say:`（答头）。Say 一出现，组行上的 running spinner 即冻结消失——
运行中的活指示凭空消失，且两条 chrome 行视觉冗余。要求：running 不消失
而转移到 Say 头行，同时把计数折叠进去；SPA 已同步（spa-say-row-running-label-merge）。

## Change Summary

- **相邻对合并**（`chat_step_render.rs`）：当 StepGroup 的**下一块即
  本 Turn 的 Say**（`Assistant`）时，独立 `❯/▸ N Steps` 行与 `❯ Say:`
  头行折叠为单行合并头 `{glyph} Say(n step{s}): <say 首行预览>`——
  `SayHeader{raw, streaming}` 由 `chat_flatten.rs::flatten_with` 按块相邻
  性构造；预览 = raw 第一个非空 trim 行，label 沿用 ok+BOLD 角色头样式。
  点击合并头 toggle 阶梯（`StepTarget::Group`，hit 记为组 target 0）。
- **running 转移**：流式中且 Say 是末块（`!done && bi+2 == len`）时，
  spinner 保留在合并头行；Done/新阶梯开启（post-Say reasoning/call 在
  Say 之下开组）后消失，与旧行为「Say 出现后后续帧不得重新激活」一致。
- **头部空行 + 正文去重**：合并头之后固定一个空行再接正文；正文首个
  非空行与 preview trim 相等时跳过（`chat_step_render.rs` 的
  `merged_say_body` 三态 Full/Skip/Hidden，done 取 markdown 行文本、
  流式取原始行，入口 `merged_say_body_decision` 供渲染与行数记账共
  用）——单行 Say/空正文整块隐藏，头部空行即整对唯一尾部空行，
  `chat_flatten.rs::last_block_ends_blank` 感知 Hidden，Done 边界不再
  叠加第二个空行；闭合对由头部空行收尾（无 ladder 尾随空行），展开
  对保持「ladder 尾部恰好一个空行」。
- **非相邻回退**：marker/subagent 隔开或无组纯文本 Say 保持旧两行布局。
- **stale 边界 marker**（`chat_steps.rs::insert_floor`）：post-Say 新阶梯
  插入时跳过 floor 处的空白 Marker（落到其后），不再把组行插进上一对
  的边界空行里（双空行根因）。
- **行数记账锁步**：`chat_headers.rs::collect_headers` 逐行镜像合并头
  与其头部空行（组 target 记合并头行、merged Say 只记去重后正文行数、
  closed 对跳 ladder 尾随空行）；`chat_tests/line_accounting.rs` 独立
  镜像同步；`copy_mode/clean.rs` 新增 `is_say_pair_header`，合并头归
  RoleHeader 剥离。

## Impact Surface

- 仅 `crates/tui`：`chat.rs`（拆出 `chat_flatten.rs`，行数 gate）/
  `chat_step_render.rs` / `chat_headers.rs` / `chat_steps.rs` /
  `chat_types.rs` / `copy_mode/clean.rs`；无接口变更，不影响 Store /
  session / 模型 context。
- 测试：更新 7 例旧布局断言（`tool_collapse` running-hint 转移、
  `step_group` 合并头、`turn_boundary`/`replay` 零点击、`thinking_state`
  头计数）；`chat_tests/say_pair.rs` 7 例（closed 头+空行+去重正文、
  点击 toggle、Ctrl+L 收起、行数记账、纯文本 Say、非相邻旧布局、
  复制剥离、边界 marker 落位）；新增 `chat_tests/say_pair_dedup.rs`
  5 例（首行不等不跳过、空正文单空行、前导空行折叠进 skip、展开对
  多行正文空行纪律、混合子轮计数 1/2/1 不累加）。

## Notes / Compatibility

- 历史会话 replay 共享同一 flatten 层，合并语义自动生效。
- Web(SPA) 同步条目见 spa-say-row-running-label-merge.md。

## Related Docs

- agents/tui/index.md（ChatBlock：相邻 Say 对合并契约）
