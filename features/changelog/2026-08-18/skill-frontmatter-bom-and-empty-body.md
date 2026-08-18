Commit: (working-tree, post-a3b1942)

# skill frontmatter 解析容错：BOM/前导空行不再吞掉 frontmatter，frontmatter-only 文件空体不注入

## 背景

两个解析缺陷叠加，会让 SKILL.md 的元数据被当成指令注入对话：
1. `split_frontmatter` 只在文件**首行恰好是 `---`** 时才识别 frontmatter。编辑器以「UTF-8 with BOM」保存（或首部有零散空行）时首行检查失败，整个文件——含 `name:`/`description:` 注释块——落入 body，随后全文注入 transcript 与 LLM payload。
2. `parse_skill` 对 body 为空的文件回退 `raw.trim()`：frontmatter-only 的 SKILL.md 会把 `---` 注释块整体当作 body 注入；即便解析正确，session 侧 `ensure_full_body_loaded` 也会记录一条 marker-only 的 `[skill loaded]` 消息。

## 变更

### core：frontmatter 解析容错
- **`crates/core/src/skill.rs`**：
  - `split_frontmatter` 先剥 UTF-8 BOM（`\u{FEFF}`）再跳过前导空行，之后才做首行 `---` 检查；无 frontmatter 时返回剥 BOM 后的原文。
  - `parse_skill` 删除 body 空时回退 `raw.trim()` 的分支——frontmatter-only 文件 body 保持空串。

### session：空体守卫
- **`crates/session/src/skill_context.rs`**：`ensure_full_body_loaded` 解析出的 body 为空白时直接返回——不记录/发送 marker-only `[skill loaded]` 消息，尾部瞬态路径指针是 skill 的唯一痕迹。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| frontmatter-only 文件 body 为空串（非原文回退） | `parse_skill_frontmatter_only_file_has_empty_body` | `crates/core/tests/skill_contract.rs` |
| BOM + 前导空行下 frontmatter 仍解析、body 仅含 fence 后文本 | `parse_skill_strips_bom_and_blank_lines_before_frontmatter` | 同上 |
| 空体 skill 不注入（无 marker-only 消息、payload 无注入） | `empty_body_skill_is_not_injected` | `crates/session/tests/skill_body_injection.rs` |
| BOM 文件端到端：仅 body 注入，frontmatter 不泄露 transcript/payload | `bom_frontmatter_end_to_end_injects_only_body` | 同上 |

- 全量回归：用户豁免当次复跑（免测提交指令；新增 4 个测试均为确定性断言，无网络/时序依赖）。
- 行数：`skill.rs` 762 ≤800、`skill_context.rs` 398 ≤800、`skill_contract.rs` 417（迭代中测试文件）、`skill_body_injection.rs` 328 ≤400。

## Impact Surface
- 用户以「UTF-8 with BOM」编辑器保存的 SKILL.md：frontmatter 恢复解析，仅正文注入。
- frontmatter-only 的 skill：激活后不再注入空壳/marker 消息，路径提醒仍在。
- 不影响：正常（无 BOM、有 body）skill 的解析与注入路径、seed 行为、CLI/TUI/Web 接口。

## Related Docs
- [agents/core](../../agents/core/index.md)
- [agents/session](../../agents/session/index.md)
- [skill 全文注入](skill-full-body-injection.md)
