Commit: (working-tree, pre-initial-commit)

# Steer 消费后行不消失修复 — seq 身份不匹配（admitted_seq vs PK seq）

## 背景
Steer（运行中插入的转向输入）消费后，TUI queue panel 里的 `↳ steer` 行**不消失**，直到整轮 `Done` 才被 `clear()` 兜底清除。问题 1（steer 回显两次）已在先前修复中移除 `chat.push_marker` 解决；问题 2（消费后不消失）虽声称已修，实则未生效。

## 诊断
### 根因：`claim_steers` 返回 `admitted_seq`，TUI 存的是 PK `seq` — 两个不同身份空间

- `admit_input` 返回主键 `seq`（`SELECT MAX(seq) FROM session_inputs`，**全局自增** PK）。TUI 在 `steer_items.push((seq, text))` 存的正是这个 PK。
- `claim_steers`（runner.rs）却返回 `i.admitted_seq`（per-session 入场序号，`COALESCE(MAX(admitted_seq),0)+1 WHERE session_id=?`，**不同列、不同语义**）。
- `SteerConsumed { seq }` 发的是 `admitted_seq`，TUI 的 `steer_items.retain(|(s,_)| s != seq)` 拿 PK 比 `admitted_seq` → **永不匹配** → 行不被移除，直到 `Done` 时 `clear()` 兜底（即原 bug 行为）。
- 根因链：`SessionInput` 结构体无 `seq`(PK) 字段，只有 `admitted_seq`；`claim_steers` 调了 `promote_inputs`（其返回值 `Vec<i64>` 正是 PK seqs）却**丢弃返回值**，转而取结构体上唯一可用的 seq-like 值 `admitted_seq`——取错了。
- 对比：queue 路径正确——`claim_next_queue` 显式 `r.get(0)?` 取 PK `seq` 返回，`QueueConsumed` 携带 PK，与 `queue_items` 存的 PK 匹配，retain 正常移除。
- 隐蔽性：单 session 首条输入时 `seq == admitted_seq == 1`，可能巧合通过；多 session 场景必现。

## 变更
### `crates/session/src/runner.rs` — `claim_steers` 返回 PK seq 而非 admitted_seq
- 捕获 `promote_inputs` 的返回值 `Vec<i64>`（PK seqs，`SELECT seq ... ORDER BY admitted_seq ASC`）——此前被丢弃。
- `pending_inputs` 与 `promote` 均按 `ORDER BY admitted_seq ASC` 返回，故两向量 1:1 对齐；`zip` 配对后 `.map(|(i, seq)| (seq, i.prompt))` 产出 `(PK seq, prompt)`。
- `SteerConsumed { seq }` 现携带 PK（`admit_input` 返回的身份），与 queue 路径的 `QueueConsumed` 模式对齐。
- doc 注释更新：`(admitted_seq, prompt)` → `(row seq, prompt)`，并说明 row seq 即 `admit_input` 返回的 PK、非 per-session `admitted_seq`。
- `SteerConsumed` variant 的 doc 注释（"Carries the consumed input's row seq"）本就正确，修复后名副其实。

### 涉及文件
- `crates/session/src/runner.rs`（新增 `SteerConsumed{seq}` variant + emit；`claim_steers` 函数体+doc，返回 `Vec<(i64,String)>`）
- `crates/session/tests/steer_followup.rs`（+2 行为测试 + `seed_session_id`/`admit_steer` 辅助）
- `crates/tui/src/chat.rs`（`apply()` match 加 `SteerConsumed{..} => {}` arm — 无通配符，新增 variant 必须补）
- `crates/web/src/handle.rs`（`from_session_event` 加 `SteerConsumed{seq}` arm → SSE `steer_consumed` + EventKind::Step）
- `crates/cli/src/run.rs`（event 回调 match 加 `SteerConsumed{..} => {}` arm）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| **SteerConsumed 携带 admit_input 返回的 PK seq（非 admitted_seq）—— 核心回归** | `steer_consumed_carries_pk_seq_not_admitted_seq` | `crates/session/tests/steer_followup.rs`（新增） |
| **多个 steer 各自携带正确的 distinct PK seq、按 admitted_seq 顺序（zip 对齐守卫）** | `multiple_steers_consumed_each_carries_distinct_pk_seq` | 同上 |

### 防伪绿验证（当次实跑，双向确认）
两个新测试在**修复前**（临时注释掉 `on_event(SessionEvent::SteerConsumed{..})` emit 行）实跑确认 **FAIL**：
- `steer_consumed_carries_pk_seq_not_admitted_seq`：`left: []`（无事件）vs `right: [4]`（PK）→ FAIL
- `multiple_steers_consumed_each_carries_distinct_pk_seq`：`left: []`（无事件）vs `right: [3, 4, 5]`（PKs）→ FAIL

修复后（恢复 emit）两测试 PASS：7 passed / 0 failed。测试通过先向**另一个 session** 注入噪声输入拉开全局 PK 与 per-session admitted_seq 的差距，确保 `seq != admitted_seq`，避免单 session 首条输入的巧合通过。

### 当次实跑证据（session crate 隔离验证）
- `cargo test -p opencoder-session --test steer_followup` → **7 passed / 0 failed**（含 2 新测试）
- `cargo test -p opencoder-session` → **15 passed / 0 failed**（steer_followup 7 + 其它 8）
- `cargo clippy -p opencoder-session --all-targets -- -D warnings` → 零警告（`Finished dev profile`）
- `cargo check -p opencoder-web -p opencoder-cli -p opencoder-tui` → 零错误（`Finished dev profile`，验证新增 variant 的消费方编译通过）
- 注：workspace 全量测试未跑（工作区有其它无关并发改动干扰），以 session crate 隔离验证为准。

## Impact Surface
- **回归守卫**：`claim_steers` 的 seq 身份契约现被两个行为测试锁定；回退为 admitted_seq 将直接导致 `cargo test` 失败。
- **消费方改动**：新增 `SteerConsumed` variant 后，chat.rs / web/handle.rs / cli/run.rs 的 exhaustive match 必须各补一个 arm（无通配符），均为无副作用的空/SSE 透传，语义不变。
- **已知遗留**：当前 TUI `steer_items` 仍为 `Vec<String>`（HEAD 既有状态，未存 seq），故端到端「消费后 retain 移除行」需 TUI 将 `steer_items` 改为 `Vec<(i64,String)>` 并加 `SteerConsumed` retain（同 `queue_items` 模式）。本次未触碰 TUI steer 存储以避免与并发 TUI 改动冲突；此为后续独立任务。
- **不影响**：queue 路径（`claim_next_queue` 本就返回 PK，未改动）。
