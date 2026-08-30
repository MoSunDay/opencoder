Commit: (working-tree, skill 注入 fallback 指针契约测试收口 + 树创伤修复)

# skill_body_injection 契约翻转收口；树级阻塞闭环

## 背景

上一轮（task-plan 去 Any Home + question 门控收敛）评审遗留 blocker：`skill_body_injection::small_skill_body_rides_payload_and_persists` 稳定失败。归属结论成立——被测实现 `skill_context.rs` 的 marker 抑制语义（并发工作流 +59 行）自洽且正确：`ensure_full_body_loaded` 在**每轮 LLM 调用前**注入 `[skill loaded]` marker，因此首轮 payload 构建时 marker 已在 transcript，`[active skill]` 尾部指针被抑制（目录为空时 tail 整体为 `None`）；旧集成测试仍断言「tail 是 payload 最后一条 user 消息且携带 `[active skill]`」，与实现新契约矛盾。单测 `tail_reminder_is_fallback_only_while_loaded_marker_present` 即该契约的权威表述。

## 收口

- **`skill_body_injection.rs` 契约翻转**：删除两条 stale 断言（`!last_user_content.contains(REV-STEP)` + `contains("[active skill]")`），替换为新契约断言——body 在整个 payload 恰好出现 1 次（只在持久化 marker 消息内，tail 绝不重复携带）+ `[active skill]` 指针在 marker 在档时全程缺席。测试的持久意图（body 随 payload 持久化、system 干净、one-shot、store round-trip、跨 turn system 字节稳定）全部保留。
- **`control_cmd.rs` 截断写修复**：`clear_context_seeds_last_say_never_directive` 缺函数收尾花括号（01:07 截断写损坏，阻塞全 workspace 编译）。本会话 01:36 补齐；所有权工作流 01:37 重写该文件收敛（现文件编译干净，clippy 全绿验证覆盖）。
- **`skill_tail_cleared_after_run_end.rs` redundant_closure 修复**：本会话 01:38 修复后，被所有权工作流 02:27 整体重写（新 fixture `preset_skill_session`）吸收取代，文件归还其所有。

## 验证边界（诚实口径）

- clippy 门禁：`cargo clippy --workspace --all-targets -- -D warnings` → **0 告警**（01:39 快照；其后所有权方又改写 skill_tail 测试与 shellguard，该两处归其复验）。
- 作用域测试（03:4x 当次复验）：`skill_body_injection` **7/0**、`question_gating` **6/0**；session/core src 自 01:07 后无变动，作用域绿稳定。
- 全量 `cargo test --workspace` 两次尝试均被环境失效化，**尚无可签收数字**：
  1. 01:40 → 跑至 session `child_cancel::parent_steer_cancels_running_child` 1 红（1087 passed）——机器负载 224+（无关 rdb 项目测试风暴）下的超时型 flake，该文件不在任何 WIP 修改面内；隔离复跑因风暴爬行被主动终止（释放 cargo 锁避免阻塞所有权方构建）。
  2. 03:20 → shellguard lib 编译失败（`crate::allowlists` 缺失等 4 错）——所有权方 03:24/03:25（**门禁运行中**）正对该 crate 做重构手术，快照踩中其中间态。
- shellguard 为孤立新 crate，无任何 crate 依赖（grep 各 Cargo.toml 证实），不传染 session/core 测试面，仅阻塞 workspace 级门禁本身。
- 另发现：01:57 启动的全量门禁日志文件在进程存活期间自 /tmp 消失，原因未能归因；后续门禁日志改落 /root。

## 移交

- 【已闭环】shellguard 换壳收口提交（5d50a8a）后已复跑双门禁并在下方测试节补最终数字（提交前终验）。
- 【移交注】`child_cancel::parent_steer_cancels_running_child` 需在低负载环境复验一次，确认非真回归（当前仅有的红发生在负载 224 风暴中）。

## 测试

- `cargo test -p opencoder-session --test skill_body_injection` → 7 passed / 0 failed（03:4x 当次）
- `cargo test -p opencoder-session --test question_gating` → 6 passed / 0 failed（03:4x 当次）
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 告警（01:39 快照）
- 全量回归：`cargo test --workspace --no-fail-fast` → **3689 passed / 0 failed**（248 个 test result 面，TEST-EXIT=0，含 `child_cancel::parent_steer_cancels_running_child` 正常负载复验通过——移交注解除）。
