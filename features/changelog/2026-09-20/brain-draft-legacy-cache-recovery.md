# Brain 工作台草稿缓存旧协议残留恢复

## 问题

brain 图契约 v2（30108c8b）后，`readDraft` 按 `plan.instances` / `schema_version: 2` 校验 localStorage 草稿，但 `draftKey` 命名空间未变。旧 SPA 时代的 v1 草稿（`plan.steps`）残留在同一 key 下时，新建/编辑计划直接命中「无法读取浏览器草稿 / 浏览器草稿格式无效，原始缓存已保留」兜底 Alert，且该 Alert 只有「重试读取」（必然再次失败）与「关闭」，`clear()` 仅在保存成功后调用——用户被死锁，无法新建草稿。

## 变更

- `crates/web/spa/src/brain/workbench/editor/draft.js`：JSON 解析失败给出中文指引文案；校验失败文案标注「可能来自旧版协议缓存，可丢弃后重新开始」，保持 fail-closed 不覆盖原文。`useDraft` 新增 `discard()`：原始缓存原文备份到 `<key>:legacy-v1` 单槽后移除原 key，并重置为干净 v2 草稿（新建为空计划、编辑为服务端版本快照），错误清空。
- `crates/web/spa/src/brain/workbench/editor.jsx`：草稿读取失败 Alert 增加「丢弃缓存并重新开始」按钮（danger），绑定 `discard`。
- 重建内嵌 SPA dist：注意 minifier 标识符命名与 antd CSS 提取顺序在本机非 bit-stable，提交产物必须来自 `scripts/check-spa-drift.sh` 同款镜像构建流程（多轮构建取稳定主流变体），否则发布门 `build.sh` 会以 `spa dist: DRIFT detected` 拒绝。本次 dist 修正链见提交 563ab375 / 07befc8e / 56675f34。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| v1 残留缓存拦截 + discard 备份与重置 | `blocks legacy-protocol v1 cache with guidance and quarantines it on discard` | `crates/web/spa/src/brain/workbench/tests/draft.test.js` |
| 损坏缓存不覆盖 + discard 备份 | `keeps damaged cache untouched until discard and preserves it as backup` | `crates/web/spa/src/brain/workbench/tests/draft.test.js` |
| UI 从 v1 残留经丢弃恢复编辑 | `recovers the blocked editor from a legacy v1 draft cache via discard` | `crates/web/spa/src/brain/workbench/tests/editor.dom.test.jsx` |

- SPA 全量：`npm test`（vitest）114 文件 / 859 用例全部通过。
- Rust 侧无代码改动（仅内嵌 dist 产物更新）：`cargo clippy --workspace --all-targets -- -D warnings` 零告警；`cargo test --workspace` 109 个套件 ok，唯一失败为 `opencoder-session --test bash_guard_plan_mode` 的 2 例，由工作区他人未提交的 shellguard 策略半成品（`bash_guard_compat_tests*.rs`）引入——干净 HEAD worktree 复核该套件 11/11 通过，与本次修复无关；受影响面 `cargo test -p opencoder-web`（内嵌新 dist 编译）全绿 EXIT=0。
- 环境备注：e2e 兄弟二进制曾因陈旧构建（server `61e760cf-dirty` / agent `cc060ae1`）导致 brain_e2e lifecycle 假失败，重建 `cargo build --workspace --bins` 后 2/2 通过；会话默认 `CARGO_TARGET_DIR` 与 repo `target/` 不一致是陈旧产物的来源。
