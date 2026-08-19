Commit: (working-tree, post-7a9f188)

# skill full-body 注入 marker 改为路径集合键，修复合并体碰撞

## 背景

复合 skill 激活（`$A $B`）时，`skill_resolve::resolve_inline_skills_with` 把各 skill 体按
`body_with_source` 拼成 `> Source: <pathA>\n\n<bodyA>\n\n> Source: <pathB>\n\n<bodyB>` 存为
skill prompt。而 `skill_context` 的 full-body 注入幂等门只认**单个**路径：
`source_path_from_body` 只取第一个 `> Source:`，`loaded_marker_matches` 只按
`[skill loaded] <path>\n` 单行前缀匹配。

## 根因

先激活 `$A`（落盘 `[skill loaded] <pathA>`，注入 bodyA）→ 再激活 `$A $B`：
`source_path_from_body` 仍返回 `<pathA>`，旧 marker 命中 → 整个注入被跳过，
**B 的 body 永远进不了上下文**。反向（`$A $B` → `$A`）同样因集合变化未被识别而错配。
幂等键是「单路径」而激活状态是「路径集合」，键空间小于状态空间。

## 变更

- `crates/session/src/skill_context.rs`：
  - 新增纯函数 `source_paths_from_body`：提取 skill prompt 中**全部**段首
    `> Source: <path>` 路径，按发现顺序返回、首现去重；路径整行解析，含空格不歧义。
    `source_path_from_body` 退化为它的首元素便捷封装（单 skill 行为不变）。
  - 新增 `full_body_marker_block(paths)`：marker 块 = 每路径一行
    `[skill loaded] <path>`，**字典序排序 + 去重**（canonical，`$A $B` 与 `$B $A`
    同块）；`full_body_marker(path)` 单行原样保留（pub 兼容）。
  - `loaded_marker_matches(messages, paths)` 签名改为路径集合：消息前导 marker 块
    解析出的路径集合必须与期望**集合精确相等**（子集/超集均不匹配 → 触发注入）；
    每行 marker 必须换行终结，保持 `/a/SKILL.md` 不误配 `/a/SKILL.md.bak` 的边界语义。
  - `ensure_full_body_loaded` 改用集合键：注入消息以 marker 块（含全部路径）开头，
    正文仍为剥掉首个 `> Source:` 前缀后的合并体（bodyA + 内嵌 `> Source: <pathB>` +
    bodyB）；超长截断的 `[INCOMPLETE SKILL]` 续读提示取首个发现的路径。
  - 旧格式（单路径 marker、集合大小 1）与新匹配完全兼容；transient tail 的
    `[active skill]` 提醒仍显示首个路径。

## 测试清单

- `skill_context::tests::compound_set_growth_reinjectects_merged_body`（先红后绿主用例：
  修复前 `left: 1, right: 2`）——`$A` 注入后切 `$A $B` 必须再次注入，且消息含双 marker
  行（排序块）与 ALPHA/BETA 两段正文。
- `compound_same_set_is_idempotent` —— 同集合（含乱序重写）不重复注入。
- `compound_set_shrink_reinjectects_single_body` —— `$A $B` → `$A` 必须重注入，
  且只含 A 的 marker 与正文。
- `full_body_marker_and_scan_semantics` —— 集合精确相等、排序 canonical、子集/超集不
  匹配、`/a/SKILL.md` vs `/a/SKILL.md.bak` 前缀防误配、未换行终结行不计、
  非 synthetic/非 user 不计、空集不匹配。
- `source_paths_from_body_variants` —— 发现顺序、去重、含空格路径、
  段中 `> Source:` 文本不计、空路径忽略。
- 回归：`crates/session/tests/skill_body_injection.rs`（单路径端到端注入/幂等/截断）
  与 `skill_context_tail.rs` 不受影响，全部保持绿。

## Impact Surface

- 复合激活后的会话：每次激活集合变化（增/删 skill）都会追加一条新的
  `[skill loaded]` 持久消息（旧行为是静默丢失新增 skill 的 body）。
- 历史 session（旧单路径 marker）resume 后：单 skill 幂等语义不变；
  旧 marker 与新集合键天然不相等时按需重注入，属预期自愈。
- pub API：`full_body_marker` 不变；`loaded_marker_matches` 签名 `&str` → `&[&str]`
  （仓库内仅 `skill_context` 自用）；`source_path_from_body` 语义不变。
- TUI `skill_display` / `app_helpers` 仍用 `source_path_from_body` 显示单路径，无需改动。

## Validation

- 先红后绿主用例（`compound_set_growth_reinjectects_merged_body`）：修复前失败信息
  `assertion left == right failed: set change must trigger a fresh injection / left: 1 / right: 2`。
- 隔离验证（HEAD=7a9f188 独立 worktree + 仅本改动）：`cargo test -p opencoder-session --lib`
  383 passed / 0 failed（其中 skill_context 13/13）；`--test skill_body_injection` 7/7、
  `--test skill_context_tail` 5/5、`--test plain_skill_prompt` 4/4、`--test skill_queue_drain` 4/4、
  `--test skill_resume` 2/2、`--test skill_mid_run` 7/7、`--test autopilot_skill_persist` 1/1；
  `cargo clippy -p opencoder-session --all-targets -- -D warnings` 零警告；rustfmt 通过。
- 共享工作树全量（与并行任务改动叠加）：lib 385/0，integration 709/0；期间出现的
  `plain_skill_prompt` / `clear_context_regression` 瞬时失败经隔离 worktree 复测证实来自
  并行任务的 in-flight 改动（runner/core），与本改动无关。

## Related Docs

- [session 模块](../../../agents/session/index.md)
- [能力地图](../../index.md)
