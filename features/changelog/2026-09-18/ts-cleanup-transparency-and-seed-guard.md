# ts -c 清理语义透明化：stopped 与空种子分开计数，空行先探 store 再删

## 背景

- 用户执行 `opencoder ts -c` 报 "removed 6 stopped session(s)"，但 `ts -l`
  只见过其中 2 行 stopped，质疑把不该清的也清了。
- ts.db WAL 取证确认：6 行全部是"不在 tmux live 集合"的注册行，其中
  2 行注册表记有任务（`ts -l` 可见的 stopped 会话），4 行是 spawn 时
  注册、从未启动的"空种子"——`ts -l` 的 `has_content` 过滤只管显示，
  cleanup 从不检查，两个口径不一致；旧输出把两类都叫
  "stopped session(s)"，用户无法解释差额。
- 潜在数据风险：若 TUI 写了 store 而 mirror 未写回 title/preview，
  有内容的会话会被当空种子静默删除。

## 变更

- `cleanup_targets`（`crates/local/src/ts/actions.rs`）返回
  `TsSweepTarget { id, has_content }`，清理路径可区分两类死行。
- 空种子行 purge 前先 `store.last_message_seq` 探测：0 条消息才删；
  有消息或探测失败一律保留（`sweep_decision` 纯函数，fail-closed），
  避免 mirror 丢失但 store 仍持真实消息的会话被静默删除。
- 输出拆分：stopped 计 "removed N stopped session(s)"，空种子计
  "swept N unused seed row(s) (never started, hidden from `ts -l`)"，
  保留行逐条打印 id8 与 `ts -r`/`ts -d` 提示；全部为空时才输出
  "no stopped sessions to clean up."。
- 本次事故对应的旧行为输出应为：
  "removed 2 stopped session(s)." + "swept 4 unused seed row(s) ..."。

## 测试

- `crates/local/src/ts/actions_tests.rs::cleanup_targets_are_dead_registry_rows_grouped_by_store`
  —— 更新为 `TsSweepTarget` 形状：DEAD_A has_content=true、EMPTY_B=false、
  live 不入选、无 store_dir 行不入组。
- `crates/local/src/ts/actions_tests.rs::sweep_decision_never_purges_a_contentless_row_with_store_messages`
  —— 有内容必删 / 空种子 0 消息删 / 空种子有消息保留 三分支契约。
- 回归：`cargo test -p opencoder-local` 156 通过 0 失败；
  `cargo clippy -p opencoder-local --all-targets` 0 警告。
