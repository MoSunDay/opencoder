Commit: (working-tree, pre-initial-commit)

# feat(session): 助手输出精简 —— 持久化/上下文方向确定性、保义归一化

## 背景

助手每轮回复在流式送达 UI 后，会被**原文逐字**持久化并在后续 turn 原样回送为
上下文。冗余的尾部空白、连续空行、首尾空行会随每轮累积，徒增**输入** token
开销却不含语义。仓库此前只有「transcript 级压缩」（`compaction.rs`：把旧历史
整段交给 small_model 摘要），缺少对**单条助手消息**的轻量、确定性、保义精简。

需求：精简输出内容，在**保留含义**前提下减少 token 开销。

## 设计

- **确定性、纯函数**，无额外 LLM 调用、无延迟、无可观察语义变化。
- 作用点选在 `runner/mod.rs`「turn 完成 → 组装 `ContentBlock::Text`」这一唯一
  汇合处：此时实时 `TextDelta` 已把原文投递给 UI，故**显示保真不受影响**；只
  精简「持久化 + 后续上下文」这份副本，单点同时削减存储与未来 input token。
- **fenced 代码块逐字节透传**（``` 与 ~~~）：代码格式、缩进、内嵌空行一律不动，
  确保「保留含义」对代码绝对成立；仅对**散文**做空格/结构归一。
- 行为由新增 `output_streamline` 配置子段驱动；clean 文本上每条规则都是 no-op。

## 变更

### `crates/core/src/config.rs`（配置 + 默认值）
- 新增 `OutputStreamlineConfig { enabled, trim_trailing, collapse_blank_lines,
  trim_outer, collapse_inline_ws }`（`#[derive(Serialize,Deserialize)]`，`Default`
  开启除 `collapse_inline_ws` 外全部——后者默认关，opt-in「激进」档）。
- `Config` 增 `pub output_streamline: OutputStreamlineConfig`（`#[serde(default)]`），
  并补 `impl Default for Config` 初值。

### `crates/session/src/streamline.rs`（新增，纯函数核心）
- `pub fn streamline(text, cfg) -> String`：编排入口。`collect_lines` 逐行按是否
  在 fenced 代码块内打标；散文行走 `streamline_prose_line`（可选 `collapse_interior_ws`
  + 去 trailing ws，**保留行首缩进**），代码行原样保留。
- `collapse_blank_runs`：2+ 连续**散文**空行收敛为 1 行；代码内空行保留并重置
  计数器，合并绝不跨代码边界。`trim_outer_blanks`：去首尾空行（保留首/尾正文缩进）。
- 14 个单元测试覆盖：clean no-op、disabled 原样、空串、去 trailing ws、空行收敛、
  首尾裁剪、保留缩进、代码围栏逐字节、~~~ 围栏、散文围绕围栏归一、inline-ws
  opt-in 保缩进、默认不折叠 inline-ws、info string 保留、无尾换行。

### `crates/session/src/runner/mod.rs`（接线，唯一持久化汇合点）
- `let (text, ..) = turn;` 之后插入
  `let text = crate::streamline::streamline(&text, &session.config.output_streamline);`
  其后 `ContentBlock::Text { text }` / `record` 路径不变。

### `crates/session/src/lib.rs` / `crates/core/src/lib.rs`
- `pub mod streamline;`；`OutputStreamlineConfig` 纳入 core 再导出。

## 涉及文件

| 文件 | 变更 |
| --- | --- |
| `crates/core/src/config.rs` | 新增 `OutputStreamlineConfig`（结构+Default）+ `Config` 字段+默认值（+~45 行） |
| `crates/session/src/streamline.rs` | 新增纯函数模块 + 14 单元测试（284 行） |
| `crates/session/src/runner/mod.rs` | 接线 1 处（+5 行） |
| `crates/session/src/lib.rs` | `pub mod streamline;`（+1 行） |
| `crates/core/src/lib.rs` | 再导出 `OutputStreamlineConfig`（+1 词） |
| `crates/session/tests/output_streamline.rs` | 新增端到端接线测试（2 用例） |

## 测试

| 测试 | 文件 | 覆盖点 |
| --- | --- | --- |
| `streamline::tests::*`（14） | `src/streamline.rs` | 各规则 + 围栏保义 + 边界 |
| `persisted_text_is_streamlined_code_preserved` | `tests/output_streamline.rs` | 全链路：mock→run→持久化 Text 已精简、围栏逐字节、严格短于原文 |
| `disabled_keeps_verbatim` | `tests/output_streamline.rs` | `enabled=false` 时原文逐字 |

## Gate

| 项 | 变更前 | 变更后 |
| --- | --- | --- |
| `cargo clippy -p opencoder-core -p opencoder-session --all-targets` | 0 警告 | 0 警告 |
| `cargo clippy --workspace --all-targets` | 仅 tui 1 预存警告 | 同（无新增） |
| `cargo test -p opencoder-session` | 全绿 | 全绿（lib 153 + 集成全过，含 +16 新增） |
| `cargo test --workspace` | 全绿 | 全绿（exit 0） |

## 风险与对齐

- **保义性**：fenced 代码逐字节透传；散文仅做空格/空行结构归一，markdown 渲染等
  价（连续空格本就坍缩为单空格）。`collapse_inline_ws` 默认关，规避行内多空格在
  罕见对齐场景的语义风险。
- **显示保真**：实时 `TextDelta` 走原文，UI 所见不受精简影响；仅持久化与后续
  上下文副本被精简（与既有 `compaction` 把旧消息替换为摘要的理念一致——存储 transcript
  本就是「受管理的表示」，非逐字日志）。
- **纯函数式**：无 class、无内部可变状态，全部 `fn(&str,&cfg)->String`。
- **行数上限**：`streamline.rs` 284 行（<400 新文件上限）。
- **默认开启**：与 `compaction.auto=true` 一致；因规则在 clean 文本上为 no-op，
  既有测试无回归（workspace 全绿）。
- **范围外**：不触及 `reasoning`/tool 结果；不做 LLM 二次摘要（那属 transcript 级
  compaction 已有能力）。subagent 摘要已有独立 240 字符上限，不在本次范围。
