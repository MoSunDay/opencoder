Commit: (working-tree, pre-initial-commit)

# feat(tui): plan-edit 模态编辑器与 plan→act handoff（附模块拆分满足行数上限）

## 背景

Plan/Act 双模式会话需要一个可编辑的「计划文本」入口：用户在 plan 模式下编辑
草稿计划，确认后切换到 act 模式时把计划作为输入喂给执行 agent。此前 TUI 缺少
这一闭环，且在实现 plan-edit 能力后，`app.rs`/`chat.rs` 因叠加新逻辑突破了仓库
**800 行迭代上限**（`rules/`：迭代中文件不得超过 800 行）。

本次完成两件事：
1. **plan-edit 模态编辑器** + plan→act handoff 闭环；
2. **模块拆分**，使 `app.rs`、`chat.rs` 回到行数上限以内（评审 4 项阻塞中的
   Gap 1 / Gap 2）。

## 变更

### `crates/tui/src/plan_edit.rs`（新增，模态编辑器核心）

- 纯数据结构 `PlanEdit { text, mode, .. }`，`PlanEditMode::{Normal, Insert}`
  （vim 风格双模）。`PlanEdit::new` 默认进入 Insert。
- `handle_plan_edit_key(pe, key, inner_w, prompt_w) -> PlanEditAction`：按键
  分派。Insert 模式过滤控制字符、`Enter` 插入换行；Normal 模式 `Esc` 退出、
  `Ctrl+C` 退出（双模均可）；返回 `PlanEditAction` 表达「提交/取消/继续」语义。
- `is_modified()` / `mode_label()` 用于渲染脏标记与 `NORMAL`/`INSERT` 标签。
- 12 个单元测试覆盖退格、换行、控制字符过滤、模式切换边界。

### `crates/tui/src/chat_types.rs` + `chat.rs`（ChatBlock::Plan）

- 新增 `ChatBlock::Plan { rendered, raw }`：计划文本作为独立渲染块，`raw` 字段
  保留可编辑原文（`rendered` 是已折行的 `Vec<Line>`）。
- `chat.rs` 内联的 `last_plan_text()`（取最近一个 Plan 块的 raw，回退到末条
  Assistant 原文）与 `update_plan_text()`（就地更新 Plan 块原文）。

### `crates/tui/src/chat_plan.rs`（新增，Gap 2 拆分）

- 将 `impl ChatView { last_plan_text, update_plan_text }` 从 `chat.rs` 抽离为
  独立模块；`lib.rs` 增 `pub mod chat_plan;`。`chat.rs` 由 >800 行降至 **767** 行。

### `crates/tui/src/key_handler.rs`（入口：Shift+I）

- plan 模式 + idle 时，`Shift+I`（大写 `'I'`）产出 `Action::EnterPlanEdit`；
  running 中、act 模式、或输入框非空时**不**进入（避免与正常 `i` 输入冲突）。
- 17 个测试覆盖各进入条件。

### `crates/tui/src/render.rs`（plan 模式渲染）

- `render()` 新增 `plan_mode: Option<&str>` 与 `pending_images: &[(String,String)]`
  形参：plan 模式下渲染模式标签边框并隐藏待发送图片预览（plan 阶段不投图）。

### `crates/tui/src/app.rs` + `app_loop.rs`（Gap 1 拆分 + handoff）

- 从 `app.rs` 抽离到 `app_loop.rs`：
  - `render_frame()`：封装 34 参 `render()` 调用 + plan-edit composer 状态提取；
  - `enter_plan_edit()`：从 `last_plan_text()` 激活 plan-edit；
  - `dispatch_plan_edit_key()`：按终端宽度计算 inner_w 后委托 `handle_plan_edit_key`；
  - `flash_visible()` + `MODE_FLASH_TICKS`：模式切换闪烁（`app.rs` 以
    `pub(crate) use app_loop::flash_visible;` 再导出，兼容 `app_tests.rs` 既有引用）。
- `app.rs` 由 >800 行降至 **786** 行，`app_loop.rs` **748** 行（均在 800 上限内）。
- plan→act handoff：Alt+Tab / Shift+Tab 切换；idle 时立即把计划喂给执行 agent，
  running 时延迟到 turn 边界；未提交输入视为纯模式切换。

## 测试清单

| 命令 | 结果 |
| --- | --- |
| `cargo build --workspace` | Finished dev profile，0 错误 |
| `cargo test -p opencoder-tui` | 全绿（lib + 集成） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 / 0 错误 |

新增/相关测试：

1. `plan_edit.rs`（12）：退格、换行、控制字符过滤、`Esc`/`Ctrl+C` 退出、模式切换。
2. `app_loop_tests.rs`（5）：plan→act idle 立即 handoff、running 延迟、未提交纯切换、
   非 plan 模式切换清空 pending。
3. `chat_tests.rs`：`last_plan_text`（空/回退 Assistant/取 Plan 块 raw/跳过空块）、
   `update_plan_text`（更新 Plan 块/无 Plan 时更新 Assistant）、plan_submitted 默认值、
   plan_handoff 生成 plan 卡 / 收尾 pending assistant。
4. `key_handler`（17）：`Shift+I` 仅在 plan 模式 idle 进入、running/act/非空输入不进入、
   小写 `i` 正常插入。
5. 集成 `plan_act_handoff.rs`（4）：无计划优雅回退、计划+输入追加、清空 transcript
   仅喂计划。
6. 集成 `plan_card_dedup.rs`（1）：handoff 不产生重复 plan 卡。
7. 集成 `plan_card_full_flow.rs`（2）：正/逆序（handoff→reset→replay）仅产出单个
   Plan 块。

## 风险与对齐

- **回归风险：低。** plan-edit 仅在 plan 模式触发，act 模式行为不变；handoff 路径
  均有集成测试覆盖。`flash_visible` 经 `pub(crate) use` 再导出，`app_tests.rs`
  既有 `crate::app::flash_visible` 引用保持可用（构建已验证）。
- **纯函数式 / 无 class**：`PlanEdit` 为纯数据结构，行为由自由函数
  `handle_plan_edit_key` 驱动；未引入类或可变内部状态，符合仓库规则。
- **行数上限**：拆分后 `app.rs`=786、`chat.rs`=767 均 ≤800；新增文件
  `chat_plan.rs`=53、`plan_edit.rs`=296 均 ≤400，满足 `rules/`。
- **范围外**：工作区同时存在 `image_render.rs`、`bg.rs`、`model_switch` 等其它
  特性的脏文件，与本特性无关，提交时需按各自 changelog 排除。
