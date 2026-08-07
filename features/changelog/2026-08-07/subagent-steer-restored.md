Commit: (working-tree, pre-initial-commit)

# 恢复 TUI subagent steer：聚焦运行中子 agent 可引导 + `>` 强制打断

## 背景
subagent steer（turn-level interrupt）在 `ef98ed5` 中实现，随后在 `a4b3395`
（移除 tool agent/浏览器/能力开关）中被**整体删除**：删除了 `subagent_input.rs`、
`KeyAction::SubagentSteer` 及 Enter 分支的 `subagent_focused` 判断、`steer_dispatch::Action::Subagent`
与 `fire_steer_interrupt` 的 `child_turn_cancels` 参数。兜底行为是聚焦运行中子 agent 按 Enter
→ steer 写进**父 session**、push 到**父面板**、然后退出子视图（app.rs），并被测试
`enter_on_focused_subagent_still_steers` 固化。底层能力（子 session `claim_steers`、
`child_turn_cancels` 注册表、`fire_child_turn_cancel`、`steer_queue_sources` 子视图路由）
全部仍在，本次按原 `ef98ed5` 设计恢复并加固。

## 变更

### 1. 恢复 `crates/tui/src/subagent_input.rs`（新文件，289 行）
- `admit_subagent_steer(store, chat, subagent_focus, text, pending_images) -> bool`：
  仅当聚焦块为 `Subagent { done: false, .. }` 时，用
  `mk_input_with_images(child_session_id, Delivery::Steer, …)` admit 到**子 session**
  （父 session、父 steer 面板、skill token、父 turn 均不触碰，不做 `resolve_persist`）；
  成功后才 push `(seq, clean)` 到**子视图** `view.steer_items`。
- **改进（对比原 ef98ed5）**：签名由 `image_uris: &[String]` 改为 `&mut pending_images`，
  对齐当前 `admit_keyboard_steer` 的「快照 + 成功才消费」约定 —— store 写失败时图片
  不被静默丢弃（新增 `store_failure_preserves_pending_images` 测试，借 FK 约束触发失败，
  免去整份 Store impl 桩）。
- `fire_subagent_turn_cancel(child_turn_cancels, chat, subagent_focus)`：按聚焦块 `id`
  （call_id）取 token 并 cancel；`done: true` 或 token 缺失则 no-op。

### 2. `key_handler.rs`
- 枚举恢复 `SubagentSteer(String)`。
- Enter 分支还原 ef98ed5 顺序：`subagent_focused → SubagentSteer`，`running → Steer`，
  idle → `Submit`。`input_disabled` 已挡住 done 子 agent 的输入。

### 3. `steer_dispatch.rs`
- 恢复 `Action::Subagent`；`resolve` 加回首参 `subagent_focused`（优先判定聚焦 → Subagent）；
- 6 个现有测试签名补首参 `false`，新增 `subagent_focused_always_targets_subagent`。

### 4. `steer_fire.rs`
- `fire_steer_interrupt(subagent_focus, running, child_cancels, child_turn_cancels, turn_cancel, chat)`：
  - `has_children = !sub_focused && running && fire_child_cancels(...)`（聚焦子 agent 时
    **不**级联取消全部子）；
  - `Action::Subagent → fire_subagent_turn_cancel(...)`（只 fire 该子自己的 turn token）；
  - `SteerParent / CancelChildrenAndSteer → fire_turn_cancel`（父路径不变）。
- 更新全部现有测试签名；新增 `focused_running_subagent_fires_only_its_own_turn_token`
  （验证父 turn_cancel 不触发、兄弟 hard-cancel 不触发）。

### 5. `app.rs`
- 注册 `#[path = "subagent_input.rs"] mod subagent_input;`；`let child_turn_cancels =
  session.child_turn_cancels.clone();`（`child_cancels` 旁）。
- 新增 `KeyAction::SubagentSteer` arm：admit 到子 session，**不退出聚焦**（可连续引导）。
- 删除 `Steer` arm 中的 `if admitted.is_some() && subagent_focus.is_some() { subagent_focus = None; }`
  （SubagentSteer 已接管聚焦态，父 steer 在聚焦时不可能再发生；顺带消除 `admitted` 中间变量）。
- `>` 分支调用改传 `subagent_focus` + `child_turn_cancels`。

### 6. `chat.rs`
- `mark_subagent_done` 清掉该子视图 `view.steer_items`：子 agent 结束前未吸收的残留 steer 行
  随一次性 `sub-*` 会话一并消失，避免滞留（`SubagentEnd` 后队列面板已回退父列表，
  残留行会不可见但陈旧）。

### 7. 测试翻转
- `app_tests/key_tests.rs`：`enter_on_focused_subagent_still_steers`（固化错误行为）
  → 翻转为 `enter_on_focused_subagent_steers_the_child`，断言 `KeyAction::SubagentSteer`。
- `key_handler_plan_edit_tests.rs`：`enter_produces_steer_when_subagent_focused` 同步翻转为
  `SubagentSteer` 断言（同批固化的错误行为）。

## 行为对齐
- 与 web 端 `post_subagent_steer`（admit 到 child_session_id + fire child turn_cancel）同构。
- `>` 只在 steer 行上渲染；聚焦运行中子 agent 时队列面板已由 `steer_queue_sources` 路由到
  子视图的 `steer_items`，`>` 命中 → `SteerSubmit` → `Action::Subagent` → fire 该子 token。

## 已知边界（与原实现一致）
- 子 agent 在 Enter 与 admit 之间恰好结束：steer 会进已结束子 session（`done:false` 门控
  只防 UI 层；残留 pending 行无害且 SubagentEnd 时会清面板）。
- 聚焦运行中子 agent 点 `>` 且子面板为空：会 fire 该子 turn token（打断但无 steer）。
  `>` 只在 steer 行上渲染，实际不可达。
- `/task` 切换后 `child_turn_cancels` 指向旧 worker 注册表 —— 与现有 `child_cancels`
  的已知局限一致，不在本次范围。

## 测试覆盖（当次实跑）
- `cargo build --workspace` → Finished，0 error
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 warning
- `cargo test --workspace` → **1932 passed / 0 failed**（127 个 test-result 行）
- `cargo test -p opencoder-tui` → 984 passed / 0 failed
- 新增 11（本轮功能）：subagent_input 8（running admit / done no-op / no-focus /
  empty-text / store 失败保图 / fire cancel / done no-op / token 缺失 no-op）、
  steer_dispatch 1（focused → Subagent）、steer_fire 1（focused 只 fire 自身 token）、
  chat 1（SubagentEnd 清残留 steer 行）。
- 翻转 2：`enter_on_focused_subagent_still_steers` → 断言 SubagentSteer、
  `enter_produces_steer_when_subagent_focused` → 断言 SubagentSteer。
- 顺带修复（fmt 事故 collateral）：恢复 `key_handler.rs` 中 tui-ghost 工作的 ALT 守卫
  （Alt+char 吞掉不插入），该守卫在 fmt 回滚时随 key_handler.rs 一起被清掉，相关
  测试由红转绿 +5；本轮总计 1916 → 1932。
