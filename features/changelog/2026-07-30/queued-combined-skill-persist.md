Commit: (working-tree, pre-initial-commit)

# fix(tui): 排队提交「{$skill} + 其他要求」时 skill 不再丢失

## 背景

用户在 turn 运行中用 **Tab 排队**提交一条同时含 `{$skill}` token 和其他文字的
输入（如 `{$repo-memory} 修复 main.rs 的 bug`）时，token 会被解析、skill body
会被注入系统提示（in-memory 生效），但 **clean 任务文字才是被入队/持久化的内容**。
skill 本身只活在 `SessionState::skill_prompt` 这把内存 `Arc<Mutex>` 里——从未写回
`sessions.skill` 列。结果：一旦 resume / 重启，`resume.rs` 只能从 `meta.skill`
（store 列）恢复 skill，而该列为 `None`，于是那条排队的「修复 bug」就在**没有 skill
的前提下**运行了。

同样的隐患也存在于 **Submit（空闲态回车）** 与 **Steer（运行中回车插队）** 两条路径：
它们都经过 `resolve_and_warn` 激活 skill，却都只走内存、不落盘。唯独技能菜单
（`KeyAction::SetSkill`）会调用 `update_session(skill=…)` 持久化——三条 token 入口
与菜单入口语义不一致，正是 bug 根因。

## 变更

### skill 持久化（镜像 SetSkill 菜单路径）

- **`crates/tui/src/skill_persist.rs`**（新文件，293 行）：两个纯函数——
  `persist_skill(store, session_id, prev, skill_handle)` 异步函数：`prev` 为提交前
  `skill_prompt` 的快照，`skill_handle` 为提交后（即 `resolve_and_warn` 写入后）的值；
  二者相等即为 no-op（纯文字提交 / 重复激活同一 skill 都不产生多余写）。store 错误
  best-effort 吞掉——内存写入已让当前 turn 立即生效。
  `resolve_persist(...)` 异步函数：把 `resolve_and_warn`（token 解析 + 内存激活）与
  `persist_skill`（落盘）合并为单一调用，消除三处调用点重复的快照+persist 样板代码。
- **`crates/tui/src/app.rs`**：在 `KeyAction::Submit` / `Steer` / `Queue` 三处统一调用
  `resolve_persist(...)`，一次完成 token 解析、内存激活、落盘。三处对称，
  确保任何「token 激活/切换 skill」的提交都把 skill 落到 store 行。
- **`crates/tui/src/lib.rs`**：注册 `pub mod skill_persist;`。

### 语义不变点（刻意保留）

- 纯 skill 提交（只有 `{$name}`、无其他文字）仍走 `skill_trigger` 文本入队/发送，
  因为触发文本本身就在 prompt 里点名了 skill——那条路径不受影响。
- skill 切换（`{$a}` → `{$b}`）会持久化新 body；重复激活同一已落盘 skill 不写。
- 内存 `skill_prompt` 的写入时机不变，当前 turn 行为完全不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| token 新激活 skill → 落盘 | `persist_skill_writes_newly_activated_skill` | `crates/tui/src/skill_persist.rs` |
| 重复激活同一 skill → 不写 | `persist_skill_skips_unchanged_skill` | `crates/tui/src/skill_persist.rs` |
| 无 token 的纯文字提交 → no-op | `persist_skill_noop_when_no_skill_token` | `crates/tui/src/skill_persist.rs` |
| skill 切换 → 持久化新 body | `persist_skill_updates_when_skill_changes` | `crates/tui/src/skill_persist.rs` |
| 回归：排队 `{$skill} 文字` skill 存活 | `persist_skill_survives_combined_queued_skill_submission` | `crates/tui/src/skill_persist.rs` |
| resolve+persist 合并：token 激活+落盘 | `resolve_persist_activates_and_stores_combined_skill_token` | `crates/tui/src/skill_persist.rs` |

- 全量回归：`cargo test --workspace` → 全绿（0 failed / 0 ignored）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`skill_persist.rs` 293 ≤ 400（新文件上限）

## Impact Surface

- **可感知影响**：resume / 重启后，先前以 `{$skill} <任务>` 形式（Submit/Steer/Queue
  任一）激活的 skill 现在会被正确恢复，排队任务会在 skill 生效下执行。
- **不影响**：纯 skill 提交（`skill_trigger`）、skill 菜单（`SetSkill`）、CLI / web /
  session 运行时主循环、store schema。
- 落盘为 best-effort；store 故障不影响当前内存 turn。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- 既有相关：`crates/session/tests/skill_resume.rs`（已落盘 skill 的 resume 契约）
